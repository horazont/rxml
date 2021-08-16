/*!
# XML 1.0 Parser
*/
use std::fmt;
#[cfg(feature = "mt")]
use std::sync::Arc;
#[cfg(not(feature = "mt"))]
use std::rc::Rc;
use std::result::Result as StdResult;
use std::collections::HashMap;
use std::collections::VecDeque;

use crate::lexer::{Token, Lexer, CodepointRead, TokenMetrics};
use crate::error::*;
use crate::strings::*;
use crate::context;

pub const XMLNS_XML: &'static CDataStr = unsafe { std::mem::transmute("http://www.w3.org/XML/1998/namespace") };

pub type QName = (Option<RcPtr<CData>>, NCName);

/// Wrapper pointer around namespace URIs
///
/// In builds with the `mt` feature, this is a [`Arc`]. In non-`mt` builds,
/// this is a [`std::rc::Rc`]
#[cfg(feature = "mt")]
pub type RcPtr<T> = Arc<T>;
/// Wrapper pointer around namespace URIs
///
/// In builds with the `mt` feature, this is a [`std::sync::Arc`].
/// In non-`mt` builds, this is a [`Rc`].
#[cfg(not(feature = "mt"))]
pub type RcPtr<T> = Rc<T>;

/**
# XML version number

Only version 1.0 is supported.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum XMLVersion {
	/// XML Version 1.0
	V1_0,
}

/// Carry measurement information about the event
///
/// In contrast to tokens (cf. [`crate::lexer::TokenMetrics`]), events are
/// always consecutive. As a caveat, any whitespace between the XML
/// declaration and the root element is attributed to the root element header.
/// While it would, semantically, make more sense to attribute it to the XML
/// declaration, this is difficult to achieve. This behaviour may change.
///
/// Because events may span multiple tokens, the same reasonable assumptions
/// which are described in [`crate::lexer::TokenMetrics::start()`] do not
/// apply here; an event may contain lots of non-token whitespace and consist
/// of many tokens. To ensure that a valid length can always be reported, only
/// the length is accounted and the start/end positions are not (as those may)
/// wrap around even while the length does not.
///
/// Event length overflows are reported as [`Error::RestrictedXml`] errors.
#[derive(Copy, Debug, Clone, PartialEq, Eq)]
pub struct EventMetrics {
	len: usize,
}

impl EventMetrics {
	/// Get the number of bytes used to generate this event.
	pub fn len(&self) -> usize {
		self.len
	}

	// Create new event metrics
	pub const fn new(len: usize) -> EventMetrics {
		EventMetrics{len: len}
	}
}

pub static ZERO_METRICS: EventMetrics = EventMetrics::new(0);

/**
# XML document parts

The term *Event* is borrowed from SAX terminology. Each [`Event`] refers to
a bit of the XML document which has been parsed.

Each event has [`EventMetrics`] attached which give information about the
number of bytes from the input stream used to generate the event.
*/
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
	/// The XML declaration.
	///
	/// As the `encoding` and `standalone` flag are forced to be `utf-8` and
	/// `yes` respectively (or absent), those values are not emitted.
	XMLDeclaration(EventMetrics, XMLVersion),
	/// The start of an XML element.
	///
	/// Contains the qualified (expanded) name of the element as pair of
	/// optional namespace URI and localname as well as a hash map of
	/// attributes.
	StartElement(EventMetrics, QName, HashMap<QName, CData>),
	/// The end of an XML element.
	///
	/// The parser enforces that start/end pairs are correctly nested, which
	/// means that there is no necessity to emit the element information
	/// again.
	EndElement(EventMetrics),
	/// Text CData.
	///
	/// References are expanded and CDATA sections processed correctly, so
	/// that the text in the event exactly corresponds to the *logical*
	/// character data.
	///
	/// This implies that the [`EventMetrics::len()`] of a text event is
	/// generally not equal to the number of bytes in the CData.
	///
	/// **Note:** Multiple consecutive `Text` events may be emitted for long
	/// sections of text or because of implementation details in the
	/// processing.
	Text(EventMetrics, CData),
}

impl Event {
	/// Return the [`EventMetrics`] of the event
	pub fn metrics(&self) -> &EventMetrics {
		match self {
			Self::XMLDeclaration(m, ..) => &m,
			Self::StartElement(m, ..) => &m,
			Self::EndElement(m, ..) => &m,
			Self::Text(m, ..) => &m,
		}
	}
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

fn add_context<T, E: ErrorWithContext>(r: StdResult<T, E>, ctx: &'static str) -> StdResult<T, E> {
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

struct ElementScratchpad {
	prefix: Option<NCName>,
	localname: NCName,
	// no hashmap here as we have to resolve the k/v pairs later on anyway
	attributes: Vec<(Option<NCName>, NCName, CData)>,
	default_namespace_decl: Option<RcPtr<CData>>,
	namespace_decls: HashMap<NCName, RcPtr<CData>>,
	// attribute scratchpad
	attrprefix: Option<NCName>,
	attrlocalname: Option<NCName>,
}

/**
# Low-level XML Parser

The [`Parser`] converts [`crate::lexer::Token`]s into [`Event`]s.

It is a low-level interface which expects to be driven from a [`TokenRead`]
source.
*/
pub struct Parser {
	ctx: RcPtr<context::Context>,
	state: State,
	fixed_xml_namespace: RcPtr<CData>,
	/// keep a stack of the element Names (i.e. (Prefix:)?Localname) as a
	/// stack for quick checks
	element_stack: Vec<Name>,
	namespace_stack: Vec<(Option<RcPtr<CData>>, HashMap<NCName, RcPtr<CData>>)>,
	element_scratchpad: Option<ElementScratchpad>,
	/// end position of the last token processed in the event
	event_last_token_end: Option<usize>,
	/// current length of the event
	event_length: usize,
	/// Internal queue for events which will be returned from the current
	/// and potentially future calls to `parse()`.
	///
	/// In contrast to the Lexer, the Parser may come into situations where
	/// multiple events need to be pushed from a single token, which is why
	/// the queue exists as a buffer.
	eventq: VecDeque<Event>,
	err: Option<Box<Error>>,
}

impl Parser {
	/// Create a new parser
	pub fn new() -> Parser {
		Self::with_context(RcPtr::new(context::Context::new()))
	}

	pub fn with_context(ctx: RcPtr<context::Context>) -> Parser {
		let xmlns = ctx.intern_cdata(XMLNS_XML.to_cdata());
		Parser{
			ctx: ctx,
			state: State::Initial,
			fixed_xml_namespace: xmlns,
			element_stack: Vec::new(),
			namespace_stack: Vec::new(),
			element_scratchpad: None,
			event_last_token_end: None,
			event_length: 0,
			eventq: VecDeque::new(),
			err: None,
		}
	}

	fn start_event(&mut self, tm: &TokenMetrics) {
		debug_assert!(self.event_last_token_end.is_none());
		self.event_last_token_end = Some(tm.end());
		self.event_length = tm.len();
	}

	fn account_token(&mut self, tm: &TokenMetrics) -> Result<usize> {
		let last_end = self.event_last_token_end.unwrap();
		self.event_length = self.event_length.checked_add(
			tm.len() + tm.start().wrapping_sub(last_end),
		).ok_or_else(||{ Error::RestrictedXml("event too long") })?;
		self.event_last_token_end = Some(tm.end());
		Ok(self.event_length)
	}

	fn finish_event(&mut self) -> EventMetrics {
		debug_assert!(self.event_last_token_end.is_some());
		let len = self.event_length;
		self.event_last_token_end = None;
		self.event_length = 0;
		EventMetrics{len: len}
	}

	fn fixed_event(&self, len: usize) -> EventMetrics {
		debug_assert!(self.event_last_token_end.is_none());
		EventMetrics{len: len}
	}

	fn read_token<'r, R: TokenRead>(&mut self, r: &'r mut R) -> Result<Option<Token>> {
		if self.event_last_token_end.is_none() {
			return r.read();
		}
		match r.read()? {
			Some(tok) => {
				self.account_token(tok.metrics())?;
				Ok(Some(tok))
			},
			None => Ok(None),
		}
	}

	/// Emit an event into the event queue.
	fn emit_event(&mut self, ev: Event) -> () {
		self.eventq.push_back(ev);
	}

	/// Poison the parser, making it return the same error for all eternity.
	fn poison(&mut self, e: Error) -> () {
		self.err = Some(Box::new(e))
	}

	/// Check if the parser is poisoned and return the corresponding error.
	fn check_poison(&self) -> Result<()> {
		if let Some(e) = self.err.as_ref() {
			Err((**e).clone())
		} else {
			Ok(())
		}
	}

	/// Initialize the element scratchpad for further processing.
	///
	/// May fail if the name is not namespace-well-formed.
	fn start_processing_element(&mut self, name: Name) -> Result<()> {
		if self.element_scratchpad.is_some() {
			panic!("element scratchpad is not None at start of element");
		}
		let (prefix, localname) = add_context(name.split_name(), ERRCTX_ELEMENT)?;
		self.element_scratchpad = Some(ElementScratchpad{
			prefix: prefix,
			localname: localname,
			attributes: Vec::new(),
			default_namespace_decl: None,
			namespace_decls: HashMap::new(),
			attrprefix: None,
			attrlocalname: None,
		});
		Ok(())
	}

	/// Lookup a namespace by prefix in the current stack of declarations.
	fn lookup_namespace<'a>(&self, prefix: Option<&'a str>) -> Option<&RcPtr<CData>> {
		match prefix {
			Some("xml") => return Some(&self.fixed_xml_namespace),
			Some(prefix) => {
				for decls in self.namespace_stack.iter().rev() {
					match decls.1.get(prefix) {
						Some(uri) => return Some(&uri),
						None => (),
					};
				};
			},
			None => {
				for decls in self.namespace_stack.iter().rev() {
					match decls.0.as_ref() {
						Some(uri) => if uri.len() > 0 {
							return Some(&uri)
						} else {
							return None
						},
						None => (),
					};
				};
			}
		}
		None
	}

	/// Complete an element and emit its event.
	///
	/// This may fail for various reasons, such as duplicate attributes or
	/// references to undeclared namespace prefixes.
	fn finalize_element(&mut self) -> Result<()> {
		let ElementScratchpad{ prefix, localname, mut attributes, default_namespace_decl, namespace_decls, attrprefix: _, attrlocalname: _ } = {
			let mut tmp: Option<ElementScratchpad> = None;
			std::mem::swap(&mut tmp, &mut self.element_scratchpad);
			tmp.unwrap()
		};
		self.namespace_stack.push((default_namespace_decl, namespace_decls));
		let (assembled_name, nsuri, localname) = match prefix {
			None => (localname.clone().as_name(), self.lookup_namespace(None), localname),
			Some(prefix) => {
				let nsuri = self.lookup_namespace(Some(&prefix)).ok_or_else(|| {
					Error::NotNamespaceWellFormed(NWFError::UndeclaredNamesacePrefix(ERRCTX_ELEMENT))
				})?;
				let assembled = prefix.add_suffix(&localname);
				(assembled, Some(nsuri), localname)
			}
		};
		let mut resolved_attributes: HashMap<QName, CData> = HashMap::new();
		for (prefix, localname, value) in attributes.drain(..) {
			let nsuri = match prefix {
				Some(prefix) => Some(self.lookup_namespace(Some(&prefix)).ok_or_else(|| {
					Error::NotNamespaceWellFormed(NWFError::UndeclaredNamesacePrefix(ERRCTX_ATTNAME))
				})?.clone()),
				None => None,
			};
			if resolved_attributes.insert((nsuri, localname), value).is_some() {
				return Err(Error::NotWellFormed(WFError::DuplicateAttribute))
			}
		}
		let nsuri = nsuri.and_then(|s| { Some(s.clone()) });
		let ev = Event::StartElement(
			self.finish_event(),
			(nsuri, localname),
			resolved_attributes,
		);
		self.emit_event(ev);
		self.element_stack.push(assembled_name);
		Ok(())
	}

	/// Pop an element off the stack and emit the corresponding EndElement
	/// event.
	fn pop_element(&mut self, em: EventMetrics) -> Result<State> {
		let ev = Event::EndElement(em);
		self.emit_event(ev);
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
		match self.read_token(r)? {
			Some(Token::XMLDeclStart(tm)) => {
				self.start_event(&tm);
				Ok(State::Decl{ substate: DeclSt::VersionName, version: None })
			},
			Some(Token::ElementHeadStart(tm, name)) => {
				self.start_event(&tm);
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
		match self.read_token(r)? {
			None => Err(Error::wfeof(ERRCTX_XML_DECL)),
			Some(Token::Name(_, name)) => match state {
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
			Some(Token::Eq(_)) => Ok(
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
			Some(Token::AttributeValue(_, v)) => match state {
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
			Some(Token::XMLDeclEnd(_)) => match state {
				DeclSt::EncodingName | DeclSt::StandaloneName | DeclSt::Close => {
					let ev = Event::XMLDeclaration(self.finish_event(), version.unwrap());
					self.emit_event(ev);
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
	fn push_attribute(&mut self, val: CData) -> Result<()> {
		let scratchpad = self.element_scratchpad.as_mut().unwrap();
		let (prefix, localname) = {
			let mut tmp_prefix: Option<NCName> = None;
			let mut tmp_localname: Option<NCName> = None;
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
					if val != XMLNS_XML {
						Err(Error::NotNamespaceWellFormed(NWFError::ReservedNamespacePrefix))
					} else {
						Ok(())
					}
				} else if val.len() == 0 {
					Err(Error::NotNamespaceWellFormed(NWFError::EmptyNamespaceUri))
				} else if scratchpad.namespace_decls.insert(localname, self.ctx.intern_cdata(val)).is_some() {
					Err(Error::NotWellFormed(WFError::DuplicateAttribute))
				} else {
					Ok(())
				}
			},
			(None, localname) if localname == "xmlns" => {
				// declares default xml namespace, move elsewhere for later lookups
				if scratchpad.default_namespace_decl.is_some() {
					Err(Error::NotWellFormed(WFError::DuplicateAttribute))
				} else {
					scratchpad.default_namespace_decl = Some(self.ctx.intern_cdata(val));
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
		match self.read_token(r)? {
			None => match state {
				ElementSt::Expected => Err(Error::wfeof(ERRCTX_DOCBEGIN)),
				_ => Err(Error::wfeof(ERRCTX_ELEMENT)),
			},
			Some(Token::ElementHeadStart(tm, name)) if state == ElementSt::Expected => {
				self.start_event(&tm);
				self.start_processing_element(name)?;
				Ok(State::Document(DocSt::Element(ElementSt::AttrName)))
			},
			Some(Token::ElementHFEnd(_)) => match state {
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
			Some(Token::ElementHeadClose(_)) => match state {
				ElementSt::AttrName => {
					self.finalize_element()?;
					Ok(self.pop_element(self.fixed_event(0))?)
				},
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_ELEMENT,
					Token::NAME_ELEMENTHEADCLOSE,
					None,
				))),
			},
			Some(Token::Name(_, name)) => match state {
				ElementSt::AttrName => {
					let (prefix, localname) = add_context(name.split_name(), ERRCTX_ATTNAME)?;
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
			Some(Token::Eq(_)) => match state {
				ElementSt::AttrEq => Ok(State::Document(DocSt::Element(ElementSt::AttrValue))),
				_ => Err(Error::NotWellFormed(WFError::UnexpectedToken(
					ERRCTX_ELEMENT,
					Token::NAME_EQ,
					None,
				))),
			},
			Some(Token::AttributeValue(_, val)) => match state {
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
			DocSt::CData => match self.read_token(r)? {
				Some(Token::Text(tm, s)) => {
					self.start_event(&tm);
					let ev = Event::Text(self.finish_event(), s);
					self.emit_event(ev);
					Ok(State::Document(DocSt::CData))
				},
				Some(Token::ElementHeadStart(tm, name)) => {
					self.start_event(&tm);
					self.start_processing_element(name)?;
					Ok(State::Document(DocSt::Element(ElementSt::AttrName)))
				},
				Some(Token::ElementFootStart(tm, name)) => {
					self.start_event(&tm);
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
			DocSt::ElementFoot => match self.read_token(r)? {
				Some(Token::ElementHFEnd(_)) => {
					let em = self.finish_event();
					self.pop_element(em)
				},
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
				State::End => match self.read_token(r)? {
					None => Ok(State::Eof),
					// whitespace after the root element is explicitly allowed
					Some(Token::Text(_, s)) if s.as_bytes().iter().all(|&c| c == b' ' || c == b'\t' || c == b'\n' || c == b'\r') => Ok(State::End),
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

	/// Release all temporary buffers
	///
	/// This is sensible to call when it is expected that no more data will be
	/// processed by the parser for a while and the memory is better used
	/// elsewhere.
	pub fn release_temporaries(&mut self) {
		self.eventq.shrink_to_fit();
		self.element_stack.shrink_to_fit();
		self.namespace_stack.shrink_to_fit();
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

	/// Return a reference to the lexer
	pub fn get_lexer(&mut self) -> &Lexer {
		&self.lexer
	}

	/// Return a mutable reference to the lexer
	pub fn get_lexer_mut(&mut self) -> &mut Lexer {
		&mut self.lexer
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
	use crate::lexer::TokenMetrics;
	use std::convert::TryInto;

	const TEST_NS: &'static str = "urn:uuid:4e1c8b65-ae37-49f8-a250-c27d52827da9";
	const TEST_NS2: &'static str = "urn:uuid:678ba034-6200-4ecd-803f-bbcbfa225236";

	const DM: TokenMetrics = TokenMetrics::new(0, 0);

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

	fn parse_err(src: &[Token]) -> Option<Error> {
		let (_, r) = parse(src);
		r.err()
	}

	#[test]
	fn parser_parse_xml_declaration() {
		let (evs, r) = parse(&[
			Token::XMLDeclStart(TokenMetrics::new(0, 1)),
			Token::Name(TokenMetrics::new(2, 3), "version".try_into().unwrap()),
			Token::Eq(TokenMetrics::new(3, 4)),
			Token::AttributeValue(TokenMetrics::new(4, 5), "1.0".try_into().unwrap()),
			Token::XMLDeclEnd(TokenMetrics::new(6, 7)),
		]);
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			Event::XMLDeclaration(em, XMLVersion::V1_0) => {
				assert_eq!(em.len(), 7);
			},
			other => panic!("unexpected event: {:?}", other),
		}
		assert!(iter.next().is_none());
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
			Token::XMLDeclStart(DM),
			Token::Name(DM, "version".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "1.0".try_into().unwrap()),
			Token::XMLDeclEnd(DM),
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
		assert!(matches!(&evs[0], Event::XMLDeclaration(EventMetrics{len: 0}, XMLVersion::V1_0)));
		assert_eq!(evs.len(), 1);
	}

	#[test]
	fn parser_parse_stepwise() {
		let toks = &[
			Token::XMLDeclStart(DM),
			Token::Name(DM, "version".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "1.0".try_into().unwrap()),
			Token::XMLDeclEnd(DM),
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
		];
		let mut reader = TokenSliceReader::new(toks);
		let mut parser = Parser::new();
		let r = parser.parse(&mut reader);
		assert!(matches!(r.unwrap().unwrap(), Event::XMLDeclaration(EventMetrics{len: 0}, XMLVersion::V1_0)));
	}

	#[test]
	fn parser_parse_element_after_xml_declaration() {
		let (evs, r) = parse(&[
			Token::XMLDeclStart(DM),
			Token::Name(DM, "version".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "1.0".try_into().unwrap()),
			Token::XMLDeclEnd(DM),
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		r.unwrap();
		assert!(matches!(&evs[1], Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(&evs[2], Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_element_without_decl() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		r.unwrap();
		assert!(matches!(&evs[0], Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(&evs[1], Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_element_with_attr() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "foo".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "bar".try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), attrs) => {
				assert_eq!(localname, "root");
				assert!(nsuri.is_none());
				assert_eq!(attrs.get(&(None, "foo".try_into().unwrap())).unwrap(), "bar");
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_element_with_xmlns() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "xmlns".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len, 0);
				assert_eq!(localname, "root");
				assert_eq!(attrs.len(), 0);
				assert_eq!(nsuri.as_ref().unwrap().as_str(), TEST_NS);
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_attribute_without_namespace_prefix() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "xmlns".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::Name(DM, "foo".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "bar".try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len, 0);
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(None, "foo".try_into().unwrap())).unwrap(), "bar");
				assert_eq!(nsuri.as_ref().unwrap().as_str(), TEST_NS);
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_attribute_with_namespace_prefix() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "xmlns:foo".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::Name(DM, "foo:bar".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "baz".try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len, 0);
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(Some(RcPtr::new(TEST_NS.try_into().unwrap())), "bar".try_into().unwrap())).unwrap(), "baz");
				assert!(nsuri.is_none());
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_xml_prefix_without_declaration() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "xml:lang".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "en".try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		r.unwrap();
		match &evs[0] {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len, 0);
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(Some(RcPtr::new("http://www.w3.org/XML/1998/namespace".try_into().unwrap())), "lang".try_into().unwrap())).unwrap(), "en");
				assert!(nsuri.is_none());
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(&evs[1], Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_reject_reserved_xmlns_prefix() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "xmlns:xmlns".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::Name(DM, "foo:bar".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "baz".try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		assert!(matches!(r.err().unwrap(), Error::NotNamespaceWellFormed(NWFError::ReservedNamespacePrefix)));
		assert_eq!(evs.len(), 0);
	}

	#[test]
	fn parser_parse_allow_xml_redeclaration() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "xmlns:xml".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "http://www.w3.org/XML/1998/namespace".try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		r.unwrap();
		assert_eq!(evs.len(), 2);
	}

	#[test]
	fn parser_parse_reject_reserved_xml_prefix_with_incorrect_value() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "xmlns:xml".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::Name(DM, "foo:bar".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "baz".try_into().unwrap()),
			Token::ElementHeadClose(DM),
		]);
		assert!(matches!(r.err().unwrap(), Error::NotNamespaceWellFormed(NWFError::ReservedNamespacePrefix)));
		assert_eq!(evs.len(), 0);
	}

	#[test]
	fn parser_parse_nested_elements() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementHeadStart(DM, "child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_mixed_content() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::Text(DM, "Hello".try_into().unwrap()),
			Token::ElementHeadStart(DM, "child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::Text(DM, "mixed".try_into().unwrap()),
			Token::ElementFootStart(DM, "child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::Text(DM, "world!".try_into().unwrap()),
			Token::ElementFootStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::Text(EventMetrics{len: 0}, t) if t == "Hello"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::Text(EventMetrics{len: 0}, t) if t == "mixed"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(matches!(iter.next().unwrap(), Event::Text(EventMetrics{len: 0}, t) if t == "world!"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_reject_mismested_elements() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementHeadStart(DM, "child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "nonchild".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		]);
		assert!(matches!(r.err().unwrap(), Error::NotWellFormed(WFError::ElementMismatch)));
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "child"));
		assert!(iter.next().is_none());
	}

	#[test]
	fn parser_parse_prefixed_elements() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "x:root".try_into().unwrap()),
			Token::Name(DM, "foo".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "bar".try_into().unwrap()),
			Token::Name(DM, "xmlns:x".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementHeadStart(DM, "child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "x:root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len, 0);
				assert_eq!(nsuri.as_ref().unwrap().as_str(), TEST_NS);
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(None, "foo".try_into().unwrap())).unwrap(), "bar");
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_nested_prefixed_elements() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "x:root".try_into().unwrap()),
			Token::Name(DM, "foo".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "bar".try_into().unwrap()),
			Token::Name(DM, "xmlns:x".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementHeadStart(DM, "x:child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "x:child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "x:root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len, 0);
				assert_eq!(nsuri.as_ref().unwrap().as_str(), TEST_NS);
				assert_eq!(localname, "root");
				assert_eq!(attrs.get(&(None, "foo".try_into().unwrap())).unwrap(), "bar");
			},
			ev => panic!("unexpected event: {:?}", ev),
		}
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.as_ref().unwrap().as_str() == TEST_NS && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_overriding_prefix_decls() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "x:root".try_into().unwrap()),
			Token::Name(DM, "xmlns:x".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementHeadStart(DM, "x:child".try_into().unwrap()),
			Token::Name(DM, "xmlns:x".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS2.try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "x:child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "x:root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.as_ref().unwrap().as_str() == TEST_NS && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.as_ref().unwrap().as_str() == TEST_NS2 && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_multiple_prefixes() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "x:root".try_into().unwrap()),
			Token::Name(DM, "xmlns:x".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::Name(DM, "xmlns:y".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS2.try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementHeadStart(DM, "y:child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "y:child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "x:root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		]);
		r.unwrap();
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.as_ref().unwrap().as_str() == TEST_NS && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.as_ref().unwrap().as_str() == TEST_NS2 && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_parse_reject_duplicate_attribute_post_ns_expansion() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "x:root".try_into().unwrap()),
			Token::Name(DM, "xmlns:x".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::Name(DM, "xmlns:y".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::Name(DM, "x:a".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "foo".try_into().unwrap()),
			Token::Name(DM, "y:a".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "foo".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "x:root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		]);
		assert!(matches!(r.err().unwrap(), Error::NotWellFormed(WFError::DuplicateAttribute)));
		assert_eq!(evs.len(), 0);
	}

	#[test]
	fn parser_parse_repeats_error_after_first_encounter() {
		let toks = &[
			Token::ElementHeadStart(DM, "x:root".try_into().unwrap()),
			Token::Name(DM, "xmlns:x".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::Name(DM, "xmlns:y".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::Name(DM, "x:a".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "foo".try_into().unwrap()),
			Token::Name(DM, "y:a".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "foo".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "x:root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		];
		let mut reader = TokenSliceReader::new(toks);
		let mut parser = Parser::new();
		let r = parser.parse(&mut reader);
		assert!(matches!(r.err().unwrap(), Error::NotWellFormed(WFError::DuplicateAttribute)));
		let r = parser.parse(&mut reader);
		assert!(matches!(r.err().unwrap(), Error::NotWellFormed(WFError::DuplicateAttribute)));
	}

	#[test]
	fn parser_rejects_empty_namespace_uri() {
		let toks = &[
			Token::ElementHeadStart(DM, "x:root".try_into().unwrap()),
			Token::Name(DM, "xmlns:x".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "x:root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		];
		let err = parse_err(toks).unwrap();
		assert!(matches!(err, Error::NotNamespaceWellFormed(NWFError::EmptyNamespaceUri)));
	}

	#[test]
	fn parser_allows_empty_namespace_uri_for_default_namespace() {
		let toks = &[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "xmlns".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		];
		let (_evs, r) = parse(toks);
		r.unwrap();
	}

	#[test]
	fn parser_handles_reset_of_default_namespace_correctly() {
		let toks = &[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::Name(DM, "xmlns".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementHeadStart(DM, "child".try_into().unwrap()),
			Token::Name(DM, "xmlns".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, "".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "child".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		];
		let (evs, r) = parse(toks);
		r.unwrap();
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.as_ref().unwrap().as_str() == TEST_NS && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "child"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
	}

	#[test]
	fn parser_rejects_undeclared_namespace_prefix_on_element() {
		let toks = &[
			Token::ElementHeadStart(DM, "y:root".try_into().unwrap()),
			Token::Name(DM, "xmlns:x".try_into().unwrap()),
			Token::Eq(DM),
			Token::AttributeValue(DM, TEST_NS.try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		];
		let err = parse_err(toks).unwrap();
		assert!(matches!(err, Error::NotNamespaceWellFormed(NWFError::UndeclaredNamesacePrefix(_))));
	}

	#[test]
	fn parser_reject_element_after_root_element() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementHeadStart(DM, "garbage".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "garbage".try_into().unwrap()),
			Token::ElementHFEnd(DM),
		]);
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(iter.next().is_none());
		match r {
			Err(Error::NotWellFormed(WFError::UnexpectedToken(_, _, _))) => (),
			other => panic!("unexpected result: {:?}", other),
		}
	}

	#[test]
	fn parser_reject_text_after_root_element() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::Text(DM, "foo".try_into().unwrap()),
		]);
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(iter.next().is_none());
		match r {
			Err(Error::NotWellFormed(WFError::UnexpectedToken(_, _, _))) => (),
			other => panic!("unexpected result: {:?}", other),
		}
	}

	#[test]
	fn parser_allow_whitespace_after_root_element() {
		let (evs, r) = parse(&[
			Token::ElementHeadStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::ElementFootStart(DM, "root".try_into().unwrap()),
			Token::ElementHFEnd(DM),
			Token::Text(DM, " \t\r\n".try_into().unwrap()),
			Token::Text(DM, "\n\r\t ".try_into().unwrap()),
		]);
		let mut iter = evs.iter();
		assert!(matches!(iter.next().unwrap(), Event::StartElement(EventMetrics{len: 0}, (nsuri, localname), _attrs) if nsuri.is_none() && localname == "root"));
		assert!(matches!(iter.next().unwrap(), Event::EndElement(EventMetrics{len: 0})));
		assert!(iter.next().is_none());
		r.unwrap();
	}
}
