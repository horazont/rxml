use std::fmt;
use std::collections::HashMap;
use std::collections::VecDeque;

use crate::lexer::{Token, Lexer, CodepointRead};
use crate::error::*;

pub const XML_NAMESPACE: &'static str = "http://www.w3.org/XML/1998/namespace";

type QName = (Option<String>, String);
type NCName = String;

/**
# XML version number

Only version 1.0 is supported.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum XMLVersion {
	/// XML Version 1.0
	V1_0,
}

/**
# XML document parts

The term *Event* is borrowed from SAX terminology. Each [`Event`] refers to
a bit of the XML document which has been parsed.
*/
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
	/// The XML declaration.
	///
	/// As the `encoding` and `standalone` flag are forced to be `utf-8` and
	/// `yes` respectively (or absent), those values are not emitted.
	XMLDeclaration(XMLVersion),
	/// The start of an XML element.
	///
	/// Contains the qualified (expanded) name of the element as pair of
	/// optional namespace URI and localname as well as a hash map of
	/// attributes.
	StartElement(QName, HashMap<QName, String>),
	/// The end of an XML element.
	///
	/// The parser enforces that start/end pairs are correctly nested, which
	/// means that there is no necessity to emit the element information
	/// again.
	EndElement,
	/// Text CData.
	///
	/// References are expanded and CDATA sections processed correctly, so
	/// that the text in the event exactly corresponds to the *logical*
	/// character data.
	///
	/// **Note:** Multiple consecutive `Text` events may be emitted for long
	/// sections of text or because of implementation details in the
	/// processing.
	Text(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum DeclSt {
	VersionName,
	VersionEq,
	VersionValue,
	EncodingName,
	EncodingEq,
	EncodingValue,
	StandaloneName,
	StandaloneEq,
	StandaloneValue,
	Close,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum ElementSt {
	// Element opener is expected here, but nothing has been done yet
	Expected,
	AttrName,
	AttrEq,
	AttrValue,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum DocSt {
	Element(ElementSt),
	CData,
	ElementFoot,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
	Initial,
	Decl{
		substate: DeclSt,
		version: Option<XMLVersion>,
	},
	Document(DocSt),
	End,
	Eof,
}

fn add_context<T>(r: Result<T>, ctx: &'static str) -> Result<T> {
	r.or_else(|e| { Err(e.with_context(ctx)) })
}

/**
# Read individual tokens from a source

Analogously to [`std::io::Read`] and intended as a wrapper around
[`crate::Lexer`], this trait provides individual tokens.
*/
pub trait TokenRead {
	/// Return a single token from the source.
	///
	/// If the EOF has been reached without errors, None is returned.
	///
	/// Lexer errors and I/O errors from the underlying data source are
	/// passed through.
	fn read(&mut self) -> Result<Option<Token>>;
}

fn split_name<'a>(mut name: String) -> Result<(Option<String>, String)> {
	let colon_pos = match name.find(':') {
		None => return Ok((None, name)),
		Some(pos) => pos,
	};
	if colon_pos == 0 || colon_pos == name.len() - 1 {
		return Err(Error::NotNamespaceWellFormed(NWFError::EmptyNamePart(
			ERRCTX_UNKNOWN,
		)));
	}

	let localname = name.split_off(colon_pos+1);
	let mut prefix = name;

	if localname.find(':').is_some() {
		// Namespaces in XML 1.0 (Third Edition) namespace-well-formed criterium 1
		return Err(Error::NotNamespaceWellFormed(NWFError::MultiColonName(
			ERRCTX_UNKNOWN,
		)));
	};

	prefix.pop();
	// do not shrink to fit here -- the prefix will be used when the element
	// is finalized to put it on the stack for quick validation of the
	// </element> token.

	debug_assert!(prefix.len() > 0);
	debug_assert!(localname.len() > 0);
	Ok((Some(prefix), localname))
}

struct ElementScratchpad {
	prefix: Option<String>,
	localname: String,
	// no hashmap here as we have to resolve the k/v pairs later on anyway
	attributes: Vec<(Option<String>, String, String)>,
	namespace_decls: HashMap<String, String>,
	// attribute scratchpad
	attrprefix: Option<String>,
	attrlocalname: Option<String>,
}

/**
# Low-level XML Parser

The [`Parser`] converts [`crate::lexer::Token`]s into [`Event`]s.

It is a low-level interface which expects to be driven from a [`TokenRead`]
source.
*/
pub struct Parser {
	state: State,
	/// keep a stack of the element Names (i.e. (Prefix:)?Localname) as a
	/// stack for quick checks
	element_stack: Vec<String>,
	namespace_stack: Vec<HashMap<String, String>>,
	element_scratchpad: Option<ElementScratchpad>,
	/// Internal queue for events which will be returned from the current
	/// and potentially future calls to `parse()`.
	///
	/// In contrast to the Lexer, the Parser may come into situations where
	/// multiple events need to be pushed from a single token, which is why
	/// the queue exists as a buffer.
	eventq: VecDeque<Event>,
	err: Option<Error>,
}

impl Parser {
	/// Create a new parser
	pub fn new() -> Parser {
		Parser{
			state: State::Initial,
			element_stack: Vec::new(),
			namespace_stack: Vec::new(),
			element_scratchpad: None,
			eventq: VecDeque::new(),
			err: None,
		}
	}

	/// Emit an event into the event queue.
	fn emit_event(&mut self, ev: Event) -> () {
		self.eventq.push_back(ev);
	}

	/// Poison the parser, making it return the same error for all eternity.
	fn poison(&mut self, e: Error) -> () {
		self.err = Some(e)
	}

	/// Check if the parser is poisoned and return the corresponding error.
	fn check_poison(&self) -> Result<()> {
		if let Some(e) = self.err.as_ref() {
			Err(e.clone())
		} else {
			Ok(())
		}
	}

	/// Initialize the element scratchpad for further processing.
	///
	/// May fail if the name is not namespace-well-formed.
	fn start_processing_element(&mut self, name: String) -> Result<()> {
		if self.element_scratchpad.is_some() {
			panic!("element scratchpad is not None at start of element");
		}
		let (prefix, localname) = add_context(split_name(name), ERRCTX_ELEMENT)?;
		self.element_scratchpad = Some(ElementScratchpad{
			prefix: prefix,
			localname: localname,
			attributes: Vec::new(),
			namespace_decls: HashMap::new(),
			attrprefix: None,
			attrlocalname: None,
		});
		Ok(())
	}

	/// Lookup a namespace by prefix in the current stack of declarations.
	fn lookup_namespace<'a>(&self, prefix: &'a str) -> Option<&str> {
		if prefix == "xml" {
			return Some(XML_NAMESPACE)
		}
		for decls in self.namespace_stack.iter().rev() {
			match decls.get(prefix) {
				Some(uri) => return Some(uri),
				None => (),
			};
		}
		None
	}

	/// Complete an element and emit its event.
	///
	/// This may fail for various reasons, such as duplicate attributes or
	/// references to undeclared namespace prefixes.
	fn finalize_element(&mut self) -> Result<()> {
		let ElementScratchpad{ prefix, localname, mut attributes, namespace_decls, attrprefix: _, attrlocalname: _ } = {
			let mut tmp: Option<ElementScratchpad> = None;
			std::mem::swap(&mut tmp, &mut self.element_scratchpad);
			tmp.unwrap()
		};
		self.namespace_stack.push(namespace_decls);
		let (assembled_name, nsuri, localname) = match prefix {
			None => (localname.clone(), self.lookup_namespace(""), localname),
			Some(mut prefix) => {
				let nsuri = self.lookup_namespace(&prefix).ok_or_else(|| {
					Error::NotNamespaceWellFormed(NWFError::UndeclaredNamesacePrefix(ERRCTX_ELEMENT))
				})?;
				prefix.push_str(":");
				prefix.push_str(&localname);
				(prefix, Some(nsuri), localname)
			}
		};
		let mut resolved_attributes: HashMap<QName, String> = HashMap::new();
		for (prefix, localname, value) in attributes.drain(..) {
			let nsuri = match prefix {
				Some(prefix) => Some(self.lookup_namespace(&prefix).ok_or_else(|| {
					Error::NotNamespaceWellFormed(NWFError::UndeclaredNamesacePrefix(ERRCTX_ATTNAME))
				})?.to_string()),
				None => None,
			};
			if resolved_attributes.insert((nsuri, localname), value).is_some() {
				return Err(Error::NotWellFormed(WFError::DuplicateAttribute))
			}
		}
		let ev = Event::StartElement(
			(nsuri.and_then(|s| { Some(s.to_string()) }), localname),
			resolved_attributes,
		);
		self.emit_event(ev);
		self.element_stack.push(assembled_name);
		Ok(())
	}

	/// Pop an element off the stack and emit the corresponding EndElement
	/// event.
	fn pop_element(&mut self) -> Result<State> {
		self.emit_event(Event::EndElement);
		debug_assert!(self.element_stack.len() > 0);
		debug_assert!(self.element_stack.len() == self.namespace_stack.len());
		self.element_stack.pop();
		self.namespace_stack.pop();
		if self.element_stack.len() == 0 {
			Ok(State::End)
		} else {
			Ok(State::Document(DocSt::CData))
		}
	}

	/// Initial parser state.
	///
	/// See [`State::Initial`].
	fn parse_initial<'r, R: TokenRead>(&mut self, r: &'r mut R) -> Result<State> {
		match r.read()? {
			Some(Token::XMLDeclStart) => Ok(State::Decl{ substate: DeclSt::VersionName, version: None }),
			Some(Token::ElementHeadStart(name)) => {
				self.start_processing_element(name)?;
				Ok(State::Document(DocSt::Element(ElementSt::AttrName)))
			},
			Some(tok) => Err(Error::NotWellFormed(WFError::UnexpectedToken(
				ERRCTX_DOCBEGIN,
				tok.name(),
				Some(&[Token::NAME_ELEMENTHEADSTART, Token::NAME_XMLDECLSTART]),
			))),
			None => Err(Error::wfeof(ERRCTX_DOCBEGIN)),
		}
	}

	/// XML declaration state.
	///
	/// See [`State::Decl`].
	fn parse_decl<'r, R: TokenRead>(&mut self, state: DeclSt, version: Option<XMLVersion>, r: &'r mut R) -> Result<State> {
		match r.read()? {
			None => Err(Error::wfeof(ERRCTX_XML_DECL)),
			Some(Token::Name(name)) => match state {
				DeclSt::VersionName => {
					if name == "version" {
						Ok(State::Decl{ substate: DeclSt::VersionEq, version: version })
					} else {
						Err(Error::NotWellFormed(WFError::InvalidSyntax("'<?xml' must be followed by version attribute")))
					}
				},
				DeclSt::EncodingName => {
					if name == "encoding" {
						Ok(State::Decl{ substate: DeclSt::EncodingEq, version: version })
					} else {
						Err(Error::NotWellFormed(WFError::InvalidSyntax("'version' attribute must be followed by '?>' or 'encoding' attribute")))
					}
				},
				DeclSt::StandaloneName => {
					if name == "standalone" {
						Ok(State::Decl{ substate: DeclSt::StandaloneEq, version: version })
					} else {
						Err(Error::NotWellFormed(WFError::InvalidSyntax("'encoding' attribute must be followed by '?>' or 'standalone' attribute")))
					}
				},
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_XML_DECL,
					Token::NAME_NAME,
					None,  // TODO: add expected tokens here
				))),
			},
			Some(Token::Eq) => Ok(
				State::Decl{
					substate: match state {
						DeclSt::VersionEq => Ok(DeclSt::VersionValue),
						DeclSt::EncodingEq => Ok(DeclSt::EncodingValue),
						DeclSt::StandaloneEq => Ok(DeclSt::StandaloneValue),
						_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
							ERRCTX_XML_DECL,
							Token::NAME_EQ,
							None,
						))),
					}?,
					version: version,
				},
			),
			Some(Token::AttributeValue(v)) => match state {
				DeclSt::VersionValue => {
					if v == "1.0" {
						Ok(State::Decl{
							substate: DeclSt::EncodingName,
							version: Some(XMLVersion::V1_0),
						})
					} else {
						Err(Error::RestrictedXml("only XML version 1.0 is allowed"))
					}
				},
				DeclSt::EncodingValue => {
					if v.eq_ignore_ascii_case("utf-8") {
						Ok(State::Decl{
							substate: DeclSt::StandaloneName,
							version: version,
						})
					} else {
						Err(Error::RestrictedXml("only utf-8 encoding is allowed"))
					}
				},
				DeclSt::StandaloneValue => {
					if v.eq_ignore_ascii_case("yes") {
						Ok(State::Decl{
							substate: DeclSt::Close,
							version: version,
						})
					} else {
						Err(Error::RestrictedXml("only standalone documents are allowed"))
					}
				},
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_XML_DECL,
					Token::NAME_ATTRIBUTEVALUE,
					None,
				))),
			},
			Some(Token::XMLDeclEnd) => match state {
				DeclSt::EncodingName | DeclSt::StandaloneName | DeclSt::Close => {
					self.emit_event(Event::XMLDeclaration(version.unwrap()));
					Ok(State::Document(DocSt::Element(ElementSt::Expected)))
				},
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_XML_DECL,
					Token::NAME_XMLDECLEND,
					None,
				))),
			},
			Some(other) => Err(Error::NotWellFormed(WFError::UnexpectedToken(
				ERRCTX_XML_DECL,
				other.name(),
				None,
			))),
		}
	}

	/// Finalize a single attribute and push it to the element scratchpad.
	///
	/// May fail for various reasons, such as attempts to redefine namespace
	/// prefixes and duplicate attributes.
	fn push_attribute(&mut self, val: String) -> Result<()> {
		let scratchpad = self.element_scratchpad.as_mut().unwrap();
		let (prefix, localname) = {
			let mut tmp_prefix: Option<String> = None;
			let mut tmp_localname: Option<String> = None;
			std::mem::swap(&mut tmp_prefix, &mut scratchpad.attrprefix);
			std::mem::swap(&mut tmp_localname, &mut scratchpad.attrlocalname);
			(tmp_prefix, tmp_localname.unwrap())
		};
		match (prefix, localname) {
			(Some(prefix), localname) if prefix == "xmlns" => {
				// declares xml namespace, move elsewhere for later lookups
				if localname == "xmlns" {
					Err(Error::NotNamespaceWellFormed(NWFError::ReservedNamespacePrefix))
				} else if localname == "xml" {
					if val != XML_NAMESPACE {
						Err(Error::NotNamespaceWellFormed(NWFError::ReservedNamespacePrefix))
					} else {
						Ok(())
					}
				} else if scratchpad.namespace_decls.insert(localname, val).is_some() {
					Err(Error::NotWellFormed(WFError::DuplicateAttribute))
				} else {
					Ok(())
				}
			},
			(None, localname) if localname == "xmlns" => {
				// declares default xml namespace, move elsewhere for later lookups
				if scratchpad.namespace_decls.insert("".to_string(), val).is_some() {
					Err(Error::NotWellFormed(WFError::DuplicateAttribute))
				} else {
					Ok(())
				}
			},
			(prefix, localname) => {
				scratchpad.attributes.push((prefix, localname, val));
				Ok(())
			},
		}
	}

	/// Element state
	///
	/// See [`DocSt::Element`].
	fn parse_element<'r, R: TokenRead>(&mut self, state: ElementSt, r: &'r mut R) -> Result<State> {
		match r.read()? {
			None => match state {
				ElementSt::Expected => Err(Error::wfeof(ERRCTX_DOCBEGIN)),
				_ => Err(Error::wfeof(ERRCTX_ELEMENT)),
			},
			Some(Token::ElementHeadStart(name)) if state == ElementSt::Expected => {
				self.start_processing_element(name)?;
				Ok(State::Document(DocSt::Element(ElementSt::AttrName)))
			},
			Some(Token::ElementHFEnd) => match state {
				ElementSt::AttrName => {
					self.finalize_element()?;
					Ok(State::Document(DocSt::CData))
				},
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_ELEMENT,
					Token::NAME_ELEMENTHEADCLOSE,
					None,
				))),
			},
			Some(Token::ElementHeadClose) => match state {
				ElementSt::AttrName => {
					self.finalize_element()?;
					Ok(self.pop_element()?)
				},
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_ELEMENT,
					Token::NAME_ELEMENTHEADCLOSE,
					None,
				))),
			},
			Some(Token::Name(name)) => match state {
				ElementSt::AttrName => {
					let (prefix, localname) = add_context(split_name(name), ERRCTX_ATTNAME)?;
					let sp = self.element_scratchpad.as_mut().unwrap();
					sp.attrprefix = prefix;
					sp.attrlocalname = Some(localname);
					Ok(State::Document(DocSt::Element(ElementSt::AttrEq)))
				}
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_ELEMENT,
					Token::NAME_NAME,
					None,
				))),
			},
			Some(Token::Eq) => match state {
				ElementSt::AttrEq => Ok(State::Document(DocSt::Element(ElementSt::AttrValue))),
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_ELEMENT,
					Token::NAME_EQ,
					None,
				))),
			},
			Some(Token::AttributeValue(val)) => match state {
				ElementSt::AttrValue => {
					self.push_attribute(val)?;
					Ok(State::Document(DocSt::Element(ElementSt::AttrName)))
				},
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_ELEMENT,
					Token::NAME_EQ,
					None,
				))),
			},
			Some(tok) => Err(Error::NotWellFormed(WFError::UnexpectedToken(
				ERRCTX_ELEMENT,
				tok.name(),
				None,
			))),
		}
	}

	/// Document content state
	///
	/// See [`State::Document`].
	fn parse_document<'r, R: TokenRead>(&mut self, state: DocSt, r: &'r mut R) -> Result<State> {
		match state {
			DocSt::Element(substate) => self.parse_element(substate, r),
			DocSt::CData => match r.read()? {
				Some(Token::Text(s)) => {
					self.emit_event(Event::Text(s));
					Ok(State::Document(DocSt::CData))
				},
				Some(Token::ElementHeadStart(name)) => {
					self.start_processing_element(name)?;
					Ok(State::Document(DocSt::Element(ElementSt::AttrName)))
				},
				Some(Token::ElementFootStart(name)) => {
					if self.element_stack[self.element_stack.len()-1] != name {
						Err(Error::NotWellFormed(WFError::ElementMismatch))
					} else {
						Ok(State::Document(DocSt::ElementFoot))
					}
				},
				Some(tok) => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_TEXT,
					tok.name(),
					Some(&[Token::NAME_TEXT, Token::NAME_ELEMENTHEADSTART, Token::NAME_ELEMENTFOOTSTART]),
				))),
				None => Err(Error::wfeof(ERRCTX_TEXT)),
			},
			DocSt::ElementFoot => match r.read()? {
				Some(Token::ElementHFEnd) => self.pop_element(),
				Some(other) => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_ELEMENT_FOOT,
					other.name(),
					Some(&[Token::NAME_ELEMENTHFEND]),
				))),
				None => Err(Error::wfeof(ERRCTX_ELEMENT_FOOT)),
			},
		}
	}

	/// Parse a single event using tokens from `r`.
	///
	/// If the end of file has been reached after a well-formed document,
	/// `None` is returned. Otherwise, if the document is still will-formed,
	/// the next [`Event`] is returned.
	///
	/// If the document violates a well-formedness constraint, the XML 1.0
	/// grammar or namespacing rules, the corresponding error is returned.
	///
	/// Errors from the token source (such as I/O errors) are forwarded.
	///
	/// **Note:** Exchanging the token source between calls to
	/// [`Parser::parse()`] is possible, but not advisible.
	pub fn parse<'r, R: TokenRead>(&mut self, r: &'r mut R) -> Result<Option<Event>> {
		self.check_poison()?;
		loop {
			if self.eventq.len() > 0 {
				return Ok(Some(self.eventq.pop_front().unwrap()))
			}

			let result = match self.state {
				State::Initial => self.parse_initial(r),
				State::Decl{ substate, version } => self.parse_decl(substate, version, r),
				State::Document(substate) => self.parse_document(substate, r),
				State::End => match r.read()? {
					None => Ok(State::Eof),
					Some(tok) => Err(Error::NotWellFormed(WFError::UnexpectedToken(
						ERRCTX_DOCEND,
						tok.name(),
						Some(&["end-of-file"]),
					))),
				},
				State::Eof => return Ok(None),
			};
			self.state = match result {
				Ok(st) => st,
				// pass through I/O errors without poisoning the parser
				Err(Error::IO(ioerr)) => return Err(Error::IO(ioerr)),
				// poison the parser for everything else to avoid emitting illegal data
				Err(other) => {
					self.poison(other.clone());
					return Err(other);
				}
			};
		}
	}
}

impl fmt::Debug for Parser {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("Parser")
			.field("state", &self.state)
			.finish()
	}
}

/// Wrapper around [`Lexer`](crate::Lexer) and
/// [`CodepointRead`](crate::lexer::CodepointRead) to provide a [`TokenRead`].
pub struct LexerAdapter<R: CodepointRead + Sized> {
	lexer: Lexer,
	src: R,
}

impl<R: CodepointRead + Sized> LexerAdapter<R> {
	/// Wraps a lexer and a codepoint source
	pub fn new(lexer: Lexer, src: R) -> Self {
		Self{
			lexer: lexer,
			src: src,
		}
	}

	/// Consume self and return the wrapped lexer and codepoint source.
	pub fn into_inner(self) -> (Lexer, R) {
		(self.lexer, self.src)
	}

	/// Return a reference to the codepoint source
	pub fn get_ref(&self) -> &R {
		&self.src
	}

	/// Return a mutable reference to the codepoint source
	pub fn get_mut(&mut self) -> &mut R {
		&mut self.src
	}
}

impl<R: CodepointRead + Sized> TokenRead for LexerAdapter<R> {
	fn read(&mut self) -> Result<Option<Token>> {
		self.lexer.lex(&mut self.src)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io;

	const TEST_NS: &'static str = "urn:uuid:4e1c8b65-ae37-49f8-a250-c27d52827da9";
	const TEST_NS2: &'static str = "urn:uuid:678ba034-6200-4ecd-803f-bbcbfa225236";

	// XXX: this should be possible without a subtype *shrug*
	struct TokenSliceReader<'x>{
		base: &'x [Token],
		offset: usize,
	}

	struct SometimesBlockingTokenSliceReader<'x>{
		base: &'x [Token],
		offset: usize,
		has_blocked: bool,
	}

	trait TokenSliceWrapper<'x> {
		fn new(src: &'x [Token]) -> Self;
	}

	impl<'x> TokenSliceWrapper<'x> for TokenSliceReader<'x> {
		fn new(src: &'x [Token]) -> TokenSliceReader<'x> {
			TokenSliceReader{
				base: src,
				offset: 0,
			}
		}
	}

	impl<'x> TokenSliceWrapper<'x> for SometimesBlockingTokenSliceReader<'x> {
		fn new(src: &'x [Token]) -> SometimesBlockingTokenSliceReader<'x> {
			SometimesBlockingTokenSliceReader{
				base: src,
				offset: 0,
				has_blocked: false,
			}
		}
	}

	impl<'x> TokenRead for TokenSliceReader<'x> {
		fn read(&mut self) -> Result<Option<Token>> {
			match self.base.get(self.offset) {
				Some(x) => {
					self.offset += 1;
					let result = x.clone();
					println!("returning token {:?}", result);
					Ok(Some(result))
				},
				None => Ok(None),
			}
		}
	}

	impl<'x> TokenRead for SometimesBlockingTokenSliceReader<'x> {
		fn read(&mut self) -> Result<Option<Token>> {
			if !self.has_blocked {
				self.has_blocked = true;
				return Err(Error::io(io::Error::new(io::ErrorKind::WouldBlock, "noise")))
			}

			match self.base.get(self.offset) {
				Some(x) => {
					self.has_blocked = false;
					self.offset += 1;
					let result = x.clone();
					println!("returning token {:?}", result);
					Ok(Some(result))
				},
				None => Ok(None),
			}
		}
	}

	fn parse_custom<'t, T: TokenSliceWrapper<'t> + TokenRead>(src: &'t [Token]) -> (Vec<Event>, Result<()>) {
		let mut sink = Vec::<Event>::new();
		let mut reader = T::new(src);
		let mut parser = Parser::new();
		loop {
			match parser.parse(&mut reader) {
				Ok(Some(ev)) => sink.push(ev),
				Ok(None) => return (sink, Ok(())),
				Err(e) => return (sink, Err(e)),
			}
		}
	}

	fn parse(src: &[Token]) -> (Vec<Event>, Result<()>) {
		parse_custom::<TokenSliceReader>(src)
	}

	#[test]
	fn parser_parse_xml_declaration() {
		let (evs, r) = parse(&[
			Token::XMLDeclStart,
			Token::Name("version".to_string()),
			Token::Eq,
			Token::AttributeValue("1.0".to_string()),
			Token::XMLDeclEnd,
		]);
		assert!(matches!(&evs[0], Event::XMLDeclaration(XMLVersion::V1_0)));
		assert_eq!(evs.len(), 1);
		assert!(matches!(r.err().unwrap(), Error::NotWellFormed(WFError::InvalidEof(ERRCTX_DOCBEGIN))));
	}

	#[test]
	fn parser_parse_wouldblock_as_first_token() {
		struct DegenerateTokenSource();

		impl TokenRead for DegenerateTokenSource {
			fn read(&mut self) -> Result<Option<Token>> {
				Err(Error::io(io::Error::new(io::ErrorKind::WouldBlock, "nevar!")))
			}
		}

		let mut reader = DegenerateTokenSource();
		let mut parser = Parser::new();
		let r = parser.parse(&mut reader);
		assert!(matches!(r.err().unwrap(), Error::IO(ioerr) if ioerr.kind() == io::ErrorKind::WouldBlock));
	}

	#[test]
	fn parser_recovers_from_wouldblock() {
		let toks = &[
			Token::XMLDeclStart,
			Token::Name("version".to_string()),
			Token::Eq,
			Token::AttributeValue("1.0".to_string()),
			Token::XMLDeclEnd,
		];
		let mut reader = SometimesBlockingTokenSliceReader::new(toks);
		let mut parser = Parser::new();
		let mut evs = Vec::new();

		loop {
			match parser.parse(&mut reader) {
				Err(Error::IO(ioerr)) if ioerr.kind() == io::ErrorKind::WouldBlock => continue,
				Err(Error::NotWellFormed(WFError::InvalidEof(ERRCTX_DOCBEGIN))) => break,
				Err(other) => panic!("unexpected error: {:?}", other),
				Ok(Some(ev)) => evs.push(ev),
				Ok(None) => panic!("unexpected eof: {:?}", parser),
			}
		}
		assert!(matches!(&evs[0], Event::XMLDeclaration(XMLVersion::V1_0)));
		assert_eq!(evs.len(), 1);
	}

	#[test]
	fn parser_parse_stepwise() {
		let toks = &[
			Token::XMLDeclStart,
			Token::Name("version".to_string()),
			Token::Eq,
			Token::AttributeValue("1.0".to_string()),
			Token::XMLDeclEnd,
			Token::ElementHeadStart("root".to_string()),
		];
		let mut reader = TokenSliceReader::new(toks);
		let mut parser = Parser::new();
		let r = parser.parse(&mut reader);
		assert!(matches!(r.unwrap().unwrap(), Event::XMLDeclaration(XMLVersion::V1_0)));
	}

	#[test]
	fn parser_parse_element_after_xml_declaration() {
		let (evs, r) = parse(&[
			Token::XMLDeclStart,
			Token::Name("version".to_string()),
			Token::Eq,
			Token::AttributeValue("1.0".to_string()),
			Token::XMLDeclEnd,
			Token::ElementHeadStart("root".to_string()),
			Token::ElementHeadClose,
		]);
		r.unwrap();
		assert!(matches!(&evs[1], Event::StartElement((nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(&evs[2], Event::EndElement));
	}

	#[test]
	fn parser_parse_element_without_decl() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::ElementHeadClose,
		]);
		r.unwrap();
		assert!(matches!(&evs[0], Event::StartElement((nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(&evs[1], Event::EndElement));
	}

	#[test]
	fn parser_parse_element_with_attr() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::Name("foo".to_string()),
			Token::Eq,
			Token::AttributeValue("bar".to_string()),
			Token::ElementHeadClose,
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement((nsuri, localname), attrs) => {
				assert_eq!(localname, "root");
				assert!(nsuri.is_none());
				assert_eq!(attrs.get(&(None, "foo".to_string())).unwrap(), "bar");
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement));
	}

	#[test]
	fn parser_parse_element_with_xmlns() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::Name("xmlns".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::ElementHeadClose,
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement((nsuri, localname), attrs) => {
				assert_eq!(localname, "root");
				assert_eq!(attrs.len(), 0);
				assert_eq!(*nsuri.as_ref().unwrap(), TEST_NS);
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement));
	}

	#[test]
	fn parser_parse_attribute_without_namespace_prefix() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::Name("xmlns".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::Name("foo".to_string()),
			Token::Eq,
			Token::AttributeValue("bar".to_string()),
			Token::ElementHeadClose,
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement((nsuri, localname), attrs) => {
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(None, "foo".to_string())).unwrap(), "bar");
				assert_eq!(*nsuri.as_ref().unwrap(), TEST_NS);
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement));
	}

	#[test]
	fn parser_parse_attribute_with_namespace_prefix() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::Name("xmlns:foo".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::Name("foo:bar".to_string()),
			Token::Eq,
			Token::AttributeValue("baz".to_string()),
			Token::ElementHeadClose,
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement((nsuri, localname), attrs) => {
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(Some(TEST_NS.to_string()), "bar".to_string())).unwrap(), "baz");
				assert!(nsuri.is_none());
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement));
	}

	#[test]
	fn parser_parse_xml_prefix_without_declaration() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::Name("xml:lang".to_string()),
			Token::Eq,
			Token::AttributeValue("en".to_string()),
			Token::ElementHeadClose,
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement((nsuri, localname), attrs) => {
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(Some("http://www.w3.org/XML/1998/namespace".to_string()), "lang".to_string())).unwrap(), "en");
				assert!(nsuri.is_none());
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement));
	}

	#[test]
	fn parser_parse_reject_reserved_xmlns_prefix() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::Name("xmlns:xmlns".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::Name("foo:bar".to_string()),
			Token::Eq,
			Token::AttributeValue("baz".to_string()),
			Token::ElementHeadClose,
		]);
		assert!(matches!(r.err().unwrap(), Error::NotNamespaceWellFormed(NWFError::ReservedNamespacePrefix)));
		assert_eq!(evs.len(), 0);
	}

	#[test]
	fn parser_parse_allow_xml_redeclaration() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::Name("xmlns:xml".to_string()),
			Token::Eq,
			Token::AttributeValue("http://www.w3.org/XML/1998/namespace".to_string()),
			Token::ElementHeadClose,
		]);
		r.unwrap();
		assert_eq!(evs.len(), 2);
	}

	#[test]
	fn parser_parse_reject_reserved_xml_prefix_with_incorrect_value() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::Name("xmlns:xml".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::Name("foo:bar".to_string()),
			Token::Eq,
			Token::AttributeValue("baz".to_string()),
			Token::ElementHeadClose,
		]);
		assert!(matches!(r.err().unwrap(), Error::NotNamespaceWellFormed(NWFError::ReservedNamespacePrefix)));
		assert_eq!(evs.len(), 0);
	}

	#[test]
	fn parser_parse_nested_elements() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::ElementHFEnd,
			Token::ElementHeadStart("child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("root".to_string()),
			Token::ElementHFEnd,
		]);
		r.unwrap();
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.is_none() && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
	}

	#[test]
	fn parser_parse_mixed_content() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::ElementHFEnd,
			Token::Text("Hello".to_string()),
			Token::ElementHeadStart("child".to_string()),
			Token::ElementHFEnd,
			Token::Text("mixed".to_string()),
			Token::ElementFootStart("child".to_string()),
			Token::ElementHFEnd,
			Token::Text("world!".to_string()),
			Token::ElementFootStart("root".to_string()),
			Token::ElementHFEnd,
		]);
		r.unwrap();
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::Text(t) if t == "Hello"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.is_none() && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::Text(t) if t == "mixed"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
		assert!(matches!(iter.next().unwrap(), Event::Text(t) if t == "world!"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
	}

	#[test]
	fn parser_reject_mismested_elements() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("root".to_string()),
			Token::ElementHFEnd,
			Token::ElementHeadStart("child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("nonchild".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("root".to_string()),
			Token::ElementHFEnd,
		]);
		assert!(matches!(r.err().unwrap(), Error::NotWellFormed(WFError::ElementMismatch)));
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.is_none() && localname == "child"));
		assert!(iter.next().is_none());
	}

	#[test]
	fn parser_parse_prefixed_elements() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("x:root".to_string()),
			Token::Name("foo".to_string()),
			Token::Eq,
			Token::AttributeValue("bar".to_string()),
			Token::Name("xmlns:x".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::ElementHFEnd,
			Token::ElementHeadStart("child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("x:root".to_string()),
			Token::ElementHFEnd,
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			Event::StartElement((nsuri, localname), attrs) => {
				assert_eq!(*nsuri.as_ref().unwrap(), TEST_NS);
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(None, "foo".to_string())).unwrap(), "bar");
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.is_none() && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
	}

	#[test]
	fn parser_parse_nested_prefixed_elements() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("x:root".to_string()),
			Token::Name("foo".to_string()),
			Token::Eq,
			Token::AttributeValue("bar".to_string()),
			Token::Name("xmlns:x".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::ElementHFEnd,
			Token::ElementHeadStart("x:child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("x:child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("x:root".to_string()),
			Token::ElementHFEnd,
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			Event::StartElement((nsuri, localname), attrs) => {
				assert_eq!(*nsuri.as_ref().unwrap(), TEST_NS);
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(None, "foo".to_string())).unwrap(), "bar");
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.as_ref().unwrap() == TEST_NS && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
	}

	#[test]
	fn parser_parse_overriding_prefix_decls() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("x:root".to_string()),
			Token::Name("xmlns:x".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::ElementHFEnd,
			Token::ElementHeadStart("x:child".to_string()),
			Token::Name("xmlns:x".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS2.to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("x:child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("x:root".to_string()),
			Token::ElementHFEnd,
		]);
		r.unwrap();
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.as_ref().unwrap() == TEST_NS && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.as_ref().unwrap() == TEST_NS2 && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
	}

	#[test]
	fn parser_parse_multiple_prefixes() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("x:root".to_string()),
			Token::Name("xmlns:x".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::Name("xmlns:y".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS2.to_string()),
			Token::ElementHFEnd,
			Token::ElementHeadStart("y:child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("y:child".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("x:root".to_string()),
			Token::ElementHFEnd,
		]);
		r.unwrap();
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.as_ref().unwrap() == TEST_NS && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement((nsuri, localname), _attrs) if nsuri.as_ref().unwrap() == TEST_NS2 && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
		assert!(matches!(iter.next().unwrap(), Event::EndElement));
	}

	#[test]
	fn parser_parse_reject_duplicate_attribute_post_ns_expansion() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart("x:root".to_string()),
			Token::Name("xmlns:x".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::Name("xmlns:y".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::Name("x:a".to_string()),
			Token::Eq,
			Token::AttributeValue("foo".to_string()),
			Token::Name("y:a".to_string()),
			Token::Eq,
			Token::AttributeValue("foo".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("x:root".to_string()),
			Token::ElementHFEnd,
		]);
		assert!(matches!(r.err().unwrap(), Error::NotWellFormed(WFError::DuplicateAttribute)));
		assert_eq!(evs.len(), 0);
	}

	#[test]
	fn parser_parse_repeats_error_after_first_encounter() {
		let toks = &[
			Token::ElementHeadStart("x:root".to_string()),
			Token::Name("xmlns:x".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::Name("xmlns:y".to_string()),
			Token::Eq,
			Token::AttributeValue(TEST_NS.to_string()),
			Token::Name("x:a".to_string()),
			Token::Eq,
			Token::AttributeValue("foo".to_string()),
			Token::Name("y:a".to_string()),
			Token::Eq,
			Token::AttributeValue("foo".to_string()),
			Token::ElementHFEnd,
			Token::ElementFootStart("x:root".to_string()),
			Token::ElementHFEnd,
		];
		let mut reader = TokenSliceReader::new(toks);
		let mut parser = Parser::new();
		let r = parser.parse(&mut reader);
		assert!(matches!(r.err().unwrap(), Error::NotWellFormed(WFError::DuplicateAttribute)));
		let r = parser.parse(&mut reader);
		assert!(matches!(r.err().unwrap(), Error::NotWellFormed(WFError::DuplicateAttribute)));
	}
}
