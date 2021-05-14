/*!
# XML 1.0 Lexer
*/
// needed for trait bounds
#[allow(unused_imports)]
use std::io;
use std::fmt;

mod read;

use crate::selectors::*;
use read::{read_validated, Endpoint, skip_matching};
use crate::error::{Error, Result, WFError};
use crate::error::*;
use crate::strings::*;

pub use read::{Utf8Char, CodepointRead, DecodingReader};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
	/* the following tokens are only emitted in the Element lexer state */
	Name(Name),
	Eq,  // =
	AttributeValue(CData),  // '...' | "..."
	XMLDeclEnd,  // ?>
	ElementHeadClose,  // />
	ElementHFEnd,  // >

	/* the following tokens are only emitted in the Content lexer state */
	Reference(char), // &...;
	XMLDeclStart,  // <?xml
	ElementHeadStart(Name),  // <
	ElementFootStart(Name),  // </

	/* the following tokens may be emitted in the CDataSection and Content
	lexer states */
	Text(CData),
}

impl Token {
	pub const NAME_NAME: &'static str = "Name";
	pub const NAME_EQ: &'static str = "'='";
	pub const NAME_ATTRIBUTEVALUE: &'static str = "AttValue";
	pub const NAME_XMLDECLEND: &'static str = "'?>'";
	pub const NAME_ELEMENTHEADCLOSE: &'static str = "'/>'";
	pub const NAME_ELEMENTHFEND: &'static str = "'>'";
	pub const NAME_REFERENCE: &'static str = "Reference";
	pub const NAME_XMLDECLSTART: &'static str = "'<?xml'";
	pub const NAME_ELEMENTHEADSTART: &'static str = "'<'";
	pub const NAME_ELEMENTFOOTSTART: &'static str = "'</'";
	pub const NAME_TEXT: &'static str = "Text";

	pub fn name(&self) -> &'static str {
		match self {
			Self::Name(..) => Self::NAME_NAME,
			Self::Eq => Self::NAME_EQ,
			Self::AttributeValue(..) => Self::NAME_ATTRIBUTEVALUE,
			Self::XMLDeclEnd => Self::NAME_XMLDECLEND,
			Self::ElementHeadClose => Self::NAME_ELEMENTHEADCLOSE,
			Self::ElementHFEnd => Self::NAME_ELEMENTHFEND,
			Self::Reference(..) => Self::NAME_REFERENCE,
			Self::XMLDeclStart => Self::NAME_XMLDECLSTART,
			Self::ElementHeadStart(..) => Self::NAME_ELEMENTHEADSTART,
			Self::ElementFootStart(..) => Self::NAME_ELEMENTFOOTSTART,
			Self::Text(..) => Self::NAME_TEXT,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CharRefRadix {
	Decimal,
	Hexadecimal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RefKind {
	Entity,
	Char(CharRefRadix),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ElementState {
	Start,
	/// Only used after <?xml
	SpaceRequired,
	Blank,
	Name,
	Eq,
	Close,
	/// Delimiter and Alphabet
	AttributeValue(char, CodepointRanges),
	/// Encountered ?
	MaybeXMLDeclEnd,
	/// Encountered /
	MaybeHeadClose,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ElementKind {
	/// standard XML element head e.g. `<foo>`
	Header,
	/// standard XML element foot e.g. `</foo>`
	Footer,
	/// XML declaration e.g. `<?xml version='1.0'?>`
	XMLDecl,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum MaybeElementState {
	Initial,
	/// Number of correct CDATA section start characters
	CDataSectionStart(usize),
	/// Number of correct XML decl start characters
	XMLDeclStart(usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ContentState {
	Initial,
	/// Encountered <
	MaybeElement(MaybeElementState),
	/// only whitespace allowed, e.g. between ?> and <
	Whitespace,
}


#[derive(Debug, Clone, Copy, PartialEq)]
enum RefReturnState {
	AttributeValue(ElementKind, char, CodepointRanges),
	Text,
}

impl RefReturnState {
	fn to_state(self) -> State {
		match self {
			Self::AttributeValue(kind, delim, selector) => State::Element{
				kind: kind,
				state: ElementState::AttributeValue(delim, selector),
			},
			Self::Text => State::Content(ContentState::Initial),
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
	Content(ContentState),
	Element{ kind: ElementKind, state: ElementState },

	/// encountered &
	Reference{ ctx: &'static str, ret: RefReturnState, kind: RefKind },

	/// Count the number of correct consecutive CDataSectionEnd characters
	/// encountered
	CDataSection(usize),

	Eof,
}

#[derive(Copy, Clone, PartialEq)]
struct DebugByte(u8);

fn escape_byte<'f>(v: u8, f: &'f mut fmt::Formatter) -> fmt::Result {
	if v >= 0x20u8 && v < 0x80u8 && v != b'\'' {
		let ch = v as char;
		write!(f, "{}", ch)
	} else {
		write!(f, "\\x{:02x}", v)
	}
}

impl fmt::Debug for DebugByte {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		f.write_str("'")?;
		escape_byte(self.0, f)?;
		f.write_str("'")?;
		Ok(())
	}
}

#[derive(Copy, Clone, PartialEq)]
struct DebugBytes<'a>(&'a [u8]);

impl<'a> fmt::Debug for DebugBytes<'a> {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		f.write_str("b\"")?;
		for b in self.0.iter() {
			escape_byte(*b, f)?;
		}
		f.write_str("\"")?;
		Ok(())
	}
}

// longest text-based entity is 4 chars
// longest valid decimal entity is log(0x10ffff, 10) => 7
// longest valid hexadecimal entity is 6.
const MAX_REFERENCE_LENGTH: usize = 8usize;

const TOK_XML_DECL_START: &'static [u8] = b"<?xml";
const TOK_XML_CDATA_START: &'static [u8] = b"<![CDATA[";
// const CLASS_XML_NAME_START_CHAR:

/// Hold options to configure a [`Lexer`].
///
/// See also [`Lexer::with_options()`].
#[derive(Debug, Clone, PartialEq, Copy)]
pub struct LexerOptions {
	/// Maximum number of bytes which can form a token.
	///
	/// This exists to limit the memory use of the Lexer for tokens where the
	/// data needs to be buffered in memory (most notably
	/// [`Token::Text`] and [`Token::AttributeValue`]).
	///
	/// If token data exceeds this limit, it depends on the token type whether
	/// a partial token is emitted or the lexing fails with
	/// [`Error::RestrictedXml`]: Text tokens are split and emitted in parts
	/// (and lexing continues), all other tokens exceeding this limit will
	/// cause an error.
	pub max_token_length: usize,
}

impl LexerOptions {
	/// Constructs default lexer options.
	///
	/// The defaults are implementation-defined and should not be relied upon.
	pub fn defaults() -> LexerOptions {
		LexerOptions{
			max_token_length: 65535,
		}
	}

	/// Set the [`LexerOptions::max_token_length`] value.
	///
	/// # Example
	///
	/// ```
	/// use rxml::{Lexer, LexerOptions};
	/// let mut lexer = Lexer::with_options(LexerOptions::defaults().max_token_length(1024));
	/// ```
	pub fn max_token_length(mut self, v: usize) -> LexerOptions {
		self.max_token_length = v;
		self
	}
}

fn resolve_named_entity(name: &str, into: &mut String) -> Result<()> {
	// amp, lt, gt, apos, quot
	match name {
		"amp" => into.push_str("&"),
		"lt" => into.push_str("<"),
		"gt" => into.push_str(">"),
		"apos" => into.push_str("'"),
		"quot" => into.push_str("\""),
		_ => return Err(Error::NotWellFormed(WFError::UndeclaredEntity)),
	};
	Ok(())
}

fn resolve_char_reference(s: &str, radix: CharRefRadix, into: &mut String) -> Result<()> {
	let radix = match radix {
		CharRefRadix::Decimal => 10,
		CharRefRadix::Hexadecimal => 16,
	};
	// cannot fail because the string is validated against the alphabet and limited in length by the lexer
	let codepoint = u32::from_str_radix(s, radix).unwrap();
	let ch = match std::char::from_u32(codepoint) {
		Some(ch) => ch,
		None => return Err(Error::NotWellFormed(WFError::InvalidChar(ERRCTX_UNKNOWN, codepoint, true))),
	};
	if contained_in_ranges(ch, VALID_XML_CDATA_RANGES) {
		into.push(ch);
		Ok(())
	} else {
		Err(Error::NotWellFormed(WFError::InvalidChar(ERRCTX_UNKNOWN, codepoint, true)))
	}
}

fn add_context<T>(r: Result<T>, ctx: &'static str) -> Result<T> {
	r.or_else(|e| { Err(e.with_context(ctx)) })
}

fn handle_eof<T>(v: Option<T>, ctx: &'static str) -> Result<T> {
	v.ok_or_else(|| {
		Error::wfeof(ctx)
	})
}

struct ST(State, Option<Token>);

impl ST {
	fn splice<'a>(self, st: &'a mut State) -> Option<Token> {
		*st = self.0;
		self.1
	}
}


/**
# Restricted XML 1.0 lexer

This lexer is able to lex a restricted subset of XML 1.0. For an overview
of the restrictions (including those imposed by [`Parser`](`crate::Parser`)),
see [`rxml`](`crate`).
*/
pub struct Lexer {
	state: State,
	scratchpad: String,
	swap: String,
	opts: LexerOptions,
	/// keep the scratchpad and state for debugging
	#[cfg(debug_assertions)]
	prev_state: (String, State),
	#[cfg(debug_assertions)]
	last_single_read: Option<Utf8Char>,
	err: Option<Error>,
}
impl Lexer {
	/// Construct a new Lexer based on [`LexerOptions::defaults()`].
	pub fn new() -> Lexer {
		Lexer::with_options(LexerOptions::defaults())
	}

	/// Construct a new Lexer with the given options.
	pub fn with_options(opts: LexerOptions) -> Lexer {
		Lexer {
			state: State::Content(ContentState::Initial),
			scratchpad: String::new(),
			swap: String::new(),
			opts: opts,
			#[cfg(debug_assertions)]
			prev_state: (String::new(), State::Content(ContentState::Initial)),
			#[cfg(debug_assertions)]
			last_single_read: None,
			err: None,
		}
	}

	fn token_length_error(&self) -> Error {
		Error::RestrictedXml("long name or reference")
	}

	fn read_validated<'r, 'x, R: CodepointRead, S: CharSelector>(&mut self, r: &'r mut R, selector: &'x S, limit: usize) -> Result<Endpoint> {
		let remaining = match limit.checked_sub(self.scratchpad.len()) {
			None => return Ok(Endpoint::Limit),
			Some(v) => v,
		};
		read_validated(
			r,
			selector,
			remaining,
			&mut self.scratchpad,
		)
	}

	fn read_single<'r, R: CodepointRead>(&mut self, r: &'r mut R) -> Result<Option<Utf8Char>> {
		let last_read = r.read()?;
		#[cfg(debug_assertions)]
		{
			self.last_single_read = last_read;
		}
		Ok(last_read)
	}

	fn drop_scratchpad(&mut self) -> Result<()> {
		self.scratchpad.clear();
		Ok(())
	}

	fn swap_scratchpad(&mut self) -> Result<()> {
		std::mem::swap(&mut self.scratchpad, &mut self.swap);
		Ok(())
	}

	fn read_swap(&mut self) -> String {
		let mut tmp = String::new();
		std::mem::swap(&mut tmp, &mut self.swap);
		tmp
	}

	fn flush_scratchpad(&mut self) -> Result<String> {
		let result = self.scratchpad.split_off(0);
		debug_assert!(self.scratchpad.len() == 0);
		Ok(result)
	}

	fn flush_scratchpad_as_name(&mut self) -> Result<Name> {
		let result = self.flush_scratchpad()?;
		#[cfg(debug_assertions)]
		{
			return Ok(Name::from_string(result)?);
		}
		#[cfg(not(debug_assertions))]
		unsafe {
			return Name::from_string_unchecked(result);
		}
	}

	fn flush_scratchpad_as_cdata(&mut self) -> Result<CData> {
		let result = self.flush_scratchpad()?;
		#[cfg(debug_assertions)]
		{
			return Ok(CData::from_string(result)?);
		}
		#[cfg(not(debug_assertions))]
		unsafe {
			return CData::from_string_unchecked(result);
		}
	}

	fn maybe_flush_scratchpad_as_text(&mut self) -> Result<Option<Token>> {
		if self.scratchpad.len() == 0 {
			Ok(None)
		} else {
			Ok(Some(Token::Text(self.flush_scratchpad_as_cdata()?)))
		}
	}

	fn lex_content<'r, R: CodepointRead>(&mut self, state: ContentState, r: &'r mut R) -> Result<ST>
	{
		match state {
			// read until next `<` or `&`, which are the only things which
			// can break us out of this state.
			ContentState::Initial => match self.read_validated(r, &CodepointRanges(VALID_XML_CDATA_RANGES_TEXT_DELIMITED), self.opts.max_token_length)? {
				Endpoint::Eof => {
					Ok(ST(
						State::Eof,
						self.maybe_flush_scratchpad_as_text()?,
					))
				},
				Endpoint::Limit => {
					Ok(ST(
						State::Content(ContentState::Initial),
						self.maybe_flush_scratchpad_as_text()?,
					))
				},
				Endpoint::Delimiter(ch) => match ch.to_char() {
					'<' => {
						Ok(ST(
							State::Content(ContentState::MaybeElement(MaybeElementState::Initial)), self.maybe_flush_scratchpad_as_text()?,
						))
					},
					'&' => {
						// We need to be careful here! First, we *have* to swap the scratchpad because
						// that is part of the contract with the Reference state. Second, we have to
						// do this *after* we "maybe" flush the scratchpad as text -- otherwise, we
						// would flush the empty text and then clobber the entity lookup.
						let tok = self.maybe_flush_scratchpad_as_text()?;
						self.swap_scratchpad()?;
						Ok(ST(
							State::Reference{
								ctx: ERRCTX_TEXT,
								ret: RefReturnState::Text,
								kind: RefKind::Entity,
							},
							tok,
						))
					},
					other => Err(Error::NotWellFormed(WFError::InvalidChar(ERRCTX_TEXT, other as u32, false))),
				},
			},
			ContentState::Whitespace => match skip_matching(r, &CLASS_XML_SPACES)? {
				(_, Endpoint::Eof) | (_, Endpoint::Limit) => {
					Ok(ST(
						State::Eof,
						None,
					))
				},
				(_, Endpoint::Delimiter(ch)) => match ch.to_char() {
					'<' => Ok(ST(
						State::Content(ContentState::MaybeElement(MaybeElementState::Initial)), None,
					)),
					other => Err(Error::NotWellFormed(WFError::UnexpectedChar(
						ERRCTX_XML_DECL_END,
						other,
						Some(&["Spaces", "<"]),
					))),
				}
			},
			ContentState::MaybeElement(MaybeElementState::Initial) => match self.read_single(r)? {
				Some(utf8ch) => match utf8ch.to_char() {
					'?' => {
						self.drop_scratchpad()?;
						Ok(ST(
							State::Content(ContentState::MaybeElement(MaybeElementState::XMLDeclStart(2))),
							None,
						))
					},
					'!' => {
						self.drop_scratchpad()?;
						Ok(ST(
							State::Content(ContentState::MaybeElement(MaybeElementState::CDataSectionStart(2))),
							None,
						))
					}
					'/' => {
						self.drop_scratchpad()?;
						Ok(ST(
							State::Element{
								kind: ElementKind::Footer,
								state: ElementState::Start,
							},
							None,
						))
					},
					ch => {
						if CLASS_XML_NAMESTART.select(ch) {
							// add the first character to the scratchpad, because read_single does not do that
							self.scratchpad.push_str(utf8ch.as_str());
							Ok(ST(
								State::Element{
									kind: ElementKind::Header,
									state: ElementState::Start,
								},
								None,
							))
						} else {
							self.drop_scratchpad()?;
							Err(Error::NotWellFormed(WFError::UnexpectedChar(ERRCTX_NAMESTART, ch, None)))
						}
					},
				},
				None => Err(Error::wfeof(ERRCTX_ELEMENT)),
			},
			ContentState::MaybeElement(MaybeElementState::XMLDeclStart(i)) => {
				debug_assert!(i < TOK_XML_DECL_START.len());
				// note: exploiting that xml decl only consists of ASCII here
				let b = handle_eof(self.read_single(r)?, ERRCTX_CDATA_SECTION_START)?.to_char() as u8;
				if b != TOK_XML_DECL_START[i] {
					return Err(Error::RestrictedXml("processing instructions"));
				}
				let next = i + 1;
				if next == TOK_XML_DECL_START.len() {
					// eliminate the `xml` from the scratchpad
					self.drop_scratchpad()?;
					Ok(ST(
						State::Element{
							kind: ElementKind::XMLDecl,
							state: ElementState::SpaceRequired,
						},
						Some(Token::XMLDeclStart),
					))
				} else {
					Ok(ST(
						State::Content(ContentState::MaybeElement(MaybeElementState::XMLDeclStart(next))),
						None,
					))
				}
			},
			ContentState::MaybeElement(MaybeElementState::CDataSectionStart(i)) => {
				debug_assert!(i < TOK_XML_CDATA_START.len());
				let b = handle_eof(self.read_single(r)?, ERRCTX_XML_DECL_START)?.to_char() as u8;
				if i == 1 && b == b'-' {
					return Err(Error::RestrictedXml("comments"));
				} else if b != TOK_XML_CDATA_START[i] {
					return Err(Error::NotWellFormed(WFError::InvalidSyntax("malformed cdata section start")));
				}
				let next = i + 1;
				if next == TOK_XML_CDATA_START.len() {
					self.drop_scratchpad()?;
					Ok(ST(
						State::CDataSection(0),
						self.maybe_flush_scratchpad_as_text()?,
					))
				} else {
					Ok(ST(
						State::Content(ContentState::MaybeElement(MaybeElementState::CDataSectionStart(next))), None,
					))
				}
			},
		}
	}

	fn lex_element_postblank(&mut self, kind: ElementKind, utf8ch: Utf8Char) -> Result<ElementState> {
		match utf8ch.to_char() {
			' ' | '\t' | '\r' | '\n' => Ok(ElementState::Blank),
			'"' => Ok(ElementState::AttributeValue('"', CodepointRanges(VALID_XML_CDATA_RANGES_ATT_QUOT_DELIMITED))),
			'\'' => Ok(ElementState::AttributeValue('\'', CodepointRanges(VALID_XML_CDATA_RANGES_ATT_APOS_DELIMITED))),
			'=' => Ok(ElementState::Eq),
			'>' => match kind {
				ElementKind::Footer | ElementKind::Header => Ok(ElementState::Close),
				ElementKind::XMLDecl => Err(Error::NotWellFormed(WFError::UnexpectedChar(ERRCTX_XML_DECL, '>', Some(&["?"])))),
			}
			'?' => match kind {
				ElementKind::XMLDecl => Ok(ElementState::MaybeXMLDeclEnd),
				_ => Err(Error::NotWellFormed(WFError::UnexpectedChar(ERRCTX_ELEMENT, '?', None))),
			},
			'/' => match kind {
				ElementKind::Header => Ok(ElementState::MaybeHeadClose),
				ElementKind::Footer => Err(Error::NotWellFormed(WFError::UnexpectedChar(ERRCTX_ELEMENT_FOOT, '/', None))),
				ElementKind::XMLDecl => Err(Error::NotWellFormed(WFError::UnexpectedChar(ERRCTX_XML_DECL, '/', None))),
			},
			ch if CLASS_XML_NAMESTART.select(ch) => {
				// write the char to scratchpad because it’ll be needed.
				self.scratchpad.push_str(utf8ch.as_str());
				Ok(ElementState::Name)
			},
			ch => Err(Error::NotWellFormed(WFError::UnexpectedChar(
				match kind {
					ElementKind::XMLDecl => ERRCTX_XML_DECL,
					_ => ERRCTX_ELEMENT,
				},
				ch,
				Some(&["whitespace", "\"", "'", "=", ">", "?", "/", "start of name"]),
			))),
		}
	}

	fn lex_element<'r, R: CodepointRead>(&mut self, kind: ElementKind, state: ElementState, r: &'r mut R) -> Result<ST> {
		match state {
			ElementState::Start => match self.read_validated(r, &CLASS_XML_NAME, self.opts.max_token_length)? {
				Endpoint::Eof => Err(Error::wfeof(ERRCTX_NAME)),
				Endpoint::Limit => Err(self.token_length_error()),
				Endpoint::Delimiter(ch) => {
					Ok(ST(
						State::Element{
							kind: kind,
							state: self.lex_element_postblank(kind, ch)?
						},
						Some(match kind {
							ElementKind::Header => Token::ElementHeadStart(self.flush_scratchpad_as_name()?),
							ElementKind::Footer => Token::ElementFootStart(self.flush_scratchpad_as_name()?),
							ElementKind::XMLDecl => panic!("invalid state"),
						}),
					))
				},
			},
			ElementState::SpaceRequired | ElementState::Blank => match skip_matching(r, &CLASS_XML_SPACES)? {
				(_, Endpoint::Eof) | (_, Endpoint::Limit) => Err(Error::wfeof(ERRCTX_ELEMENT)),
				(nmatching, Endpoint::Delimiter(ch)) => {
					if state == ElementState::SpaceRequired && nmatching == 0 {
						Err(Error::NotWellFormed(WFError::InvalidSyntax(
							"space required after xml declaration header",
						)))
					} else {
						Ok(ST(
							State::Element{
								kind: kind,
								state: self.lex_element_postblank(kind, ch)?,
							},
							None,
						))
					}
				},
			},
			ElementState::Name => match self.read_validated(r, &CLASS_XML_NAME, self.opts.max_token_length)? {
				Endpoint::Eof => Err(Error::wfeof(ERRCTX_NAME)),
				Endpoint::Limit => Err(self.token_length_error()),
				Endpoint::Delimiter(ch) => {
					Ok(ST(
						State::Element{
							kind: kind,
							state: self.lex_element_postblank(kind, ch)?
						},
						Some(Token::Name(self.flush_scratchpad_as_name()?)),
					))
				},
			},
			// XML 1.0 §2.3 [10] AttValue
			ElementState::AttributeValue(delim, selector) => match self.read_validated(r, &selector, self.opts.max_token_length)? {
				Endpoint::Eof => Err(Error::wfeof(ERRCTX_ATTVAL)),
				Endpoint::Limit => Err(self.token_length_error()),
				Endpoint::Delimiter(utf8ch) => match utf8ch.to_char() {
					'<' => Err(Error::NotWellFormed(WFError::UnexpectedChar(ERRCTX_ATTVAL, '<', None))),
					'&' => {
						// must swap scratchpad here to avoid clobbering the
						// attribute value during entity read
						self.swap_scratchpad()?;
						Ok(ST(
							State::Reference{
								ctx: ERRCTX_ATTVAL,
								ret: RefReturnState::AttributeValue(
									kind,
									delim,
									selector,
								),
								kind: RefKind::Entity,
							}, None
						))
					},
					d if d == delim => Ok(ST(
						State::Element{
							kind: kind,
							state: ElementState::Blank
						},
						Some(Token::AttributeValue(self.flush_scratchpad_as_cdata()?)),
					)),
					other => Err(Error::NotWellFormed(WFError::InvalidChar(
						ERRCTX_ATTVAL,
						other as u32,
						false,
					)))
				},
			},
			ElementState::MaybeXMLDeclEnd => match self.read_single(r)? {
				Some(ch) if ch.to_char() == '>' => {
					self.drop_scratchpad()?;
					Ok(ST(
						State::Content(ContentState::Whitespace),
						Some(Token::XMLDeclEnd),
					))
				},
				Some(ch) => Err(Error::NotWellFormed(WFError::UnexpectedChar(
					ERRCTX_XML_DECL_END,
					ch.to_char(),
					Some(&[">"]),
				))),
				None => Err(Error::wfeof(ERRCTX_XML_DECL_END)),
			},
			ElementState::MaybeHeadClose => match self.read_single(r)? {
				Some(ch) if ch.to_char() == '>' => {
					self.drop_scratchpad()?;
					Ok(ST(
						State::Content(ContentState::Initial),
						Some(Token::ElementHeadClose),
					))
				},
				Some(ch) => Err(Error::NotWellFormed(WFError::UnexpectedChar(
					ERRCTX_ELEMENT_CLOSE,
					ch.to_char(),
					Some(&[">"]),
				))),
				None => Err(Error::wfeof(ERRCTX_ELEMENT_CLOSE)),
			},
			// do NOT read anything here; this state is entered when
			// another state has read a '='. We can always transition to
			// Blank afterward, as that will read the next char and decide
			// (and potentially scratchpad) correctly.
			ElementState::Eq => Ok(ST(
				State::Element{
					kind: kind,
					state: ElementState::Blank,
				},
				Some(Token::Eq),
			)),
			// like with Eq, no read here
			ElementState::Close => Ok(ST(
				State::Content(ContentState::Initial),
				Some(Token::ElementHFEnd),
			)),
		}
	}

	fn lex_reference<'r, R: CodepointRead>(&mut self, ctx: &'static str, ret: RefReturnState, kind: RefKind, r: &'r mut R) -> Result<ST> {
		let result = match kind {
			RefKind::Entity => self.read_validated(r, &CLASS_XML_NAME, MAX_REFERENCE_LENGTH)?,
			RefKind::Char(CharRefRadix::Decimal) => self.read_validated(r, &CLASS_XML_DECIMAL_DIGITS, MAX_REFERENCE_LENGTH)?,
			RefKind::Char(CharRefRadix::Hexadecimal) => self.read_validated(r, &CLASS_XML_HEXADECIMAL_DIGITS, MAX_REFERENCE_LENGTH)?,
		};
		let result = match result {
			Endpoint::Eof => return Err(Error::wfeof(ERRCTX_REF)),
			Endpoint::Limit => return Err(Error::NotWellFormed(WFError::UndeclaredEntity)),
			Endpoint::Delimiter(utf8ch) => match utf8ch.to_char() {
				'#' => {
					if self.scratchpad.len() > 0 {
						Err('#')
					} else {
						match kind {
							RefKind::Entity => {
								return Ok(ST(
									State::Reference{
										ctx: ctx,
										ret: ret,
										kind: RefKind::Char(CharRefRadix::Decimal),
									},
									None,
								))
							},
							_ => Err('#'),
						}
					}
				},
				'x' => {
					if self.scratchpad.len() > 0 {
						Err('x')
					} else {
						match kind {
							RefKind::Char(CharRefRadix::Decimal) => {
								return Ok(ST(
									State::Reference{
										ctx: ctx,
										ret: ret,
										kind: RefKind::Char(CharRefRadix::Hexadecimal),
									},
									None,
								))
							},
							_ => Err('x'),
						}
					}
				},
				';' => {
					if self.scratchpad.len() == 0 {
						return Err(Error::NotWellFormed(WFError::InvalidSyntax("empty reference")));
					}
					// return to main scratchpad
					self.swap_scratchpad()?;
					// the entity reference is now in the swap (which we have to clear now, too)
					let entity = self.read_swap();
					match kind {
						RefKind::Entity => Ok(add_context(resolve_named_entity(entity.as_str(), &mut self.scratchpad), ctx)?),
						RefKind::Char(radix) => Ok(add_context(resolve_char_reference(entity.as_str(), radix, &mut self.scratchpad), ctx)?),
					}
				}
				c => Err(c),
			}
		};
		match result {
			Ok(_) => Ok(ST(ret.to_state(), None)),
			Err(ch) => return Err(Error::NotWellFormed(WFError::UnexpectedChar(
				ERRCTX_REF,
				ch,
				Some(&[";"]),
			))),
		}
	}

	fn lex_cdata_section<'r, R: CodepointRead>(&mut self, nend: usize, r: &'r mut R) -> Result<ST> {
		match nend {
			0 => match self.read_validated(r, &CodepointRanges(VALID_XML_CDATA_RANGES_CDATASECTION_DELIMITED), self.opts.max_token_length)? {
				Endpoint::Eof => Err(Error::wfeof(ERRCTX_CDATA_SECTION)),
				Endpoint::Limit => Ok(ST(
					State::CDataSection(0),
					self.maybe_flush_scratchpad_as_text()?,
				)),
				Endpoint::Delimiter(_) => {
					// we know that the delimiter is ']' -> transition into the "first delimiter found" state
					Ok(ST(
						State::CDataSection(1),
						None,
					))
				},
			},
			1 => {
				// we found one ']', so we read the next thing and check if it is another ']'
				let utf8ch = handle_eof(self.read_single(r)?, ERRCTX_CDATA_SECTION)?;
				let ch = utf8ch.to_char();
				if ch == ']' {
					// one step closer
					Ok(ST(
						State::CDataSection(2),
						None,
					))
				} else {
					// not the ']' we were looking for -- flush the "buffered" non-delimiter to the scratchpad and continue
					self.scratchpad.push_str("]");
					self.scratchpad.push_str(utf8ch.as_str());
					Ok(ST(
						State::CDataSection(0),
						None,
					))
				}
			},
			2 => {
				// we read two consecutive ']', so we now need to check for a single '>'.
				let utf8ch = handle_eof(self.read_single(r)?, ERRCTX_CDATA_SECTION)?;
				let ch = utf8ch.to_char();
				match ch {
					// end of CDATA section, flush the scratchpad
					'>' => Ok(ST(
						State::Content(ContentState::Initial),
						self.maybe_flush_scratchpad_as_text()?,
					)),
					']' => {
						// could be either the end of the cdata section, or a sequence of closing square brackets
						// we have to add one `]` to the scratchpad and then continue in the same state
						self.scratchpad.push_str("]");
						Ok(ST(
							State::CDataSection(2),
							None,
						))
					},
					_ => {
						// not a `>` and not a `]`, we thus have read a `]]` without `>` -> add it to the scratchpad and continue lexing the CDATA section
						self.scratchpad.push_str("]]");
						self.scratchpad.push_str(utf8ch.as_str());
						// continue with the CDATA section as before without unnecessary flush
						Ok(ST(
							State::CDataSection(0),
							None,
						))
					},
				}
			},
			_ => panic!("invalid state"),
		}
	}

	/// Lex codepoints from the reader until either an error occurs, a valid
	/// token is produced or a valid end-of-file situation is encountered.
	///
	/// If an [`Error::IO`] is encountered, that error is just returned
	/// directly and it is valid to call `lex()` again. However, if an error
	/// which is not [`Error::IO`] is encountered, the Lexer is poisoned and
	/// all future calls to `lex()` will return a clone of the same error.
	///
	/// **Note**: While it is possible to swap the reader between calls, that
	/// is highly not recommended as the lexer state may depend on lookaheads.
	pub fn lex<'r, R: CodepointRead>(&mut self, r: &'r mut R) -> Result<Option<Token>>
	{
		if let Some(e) = self.err.as_ref() {
			return Err(e.clone())
		}

		loop {
			let stresult = match self.state {
				State::Content(substate) => self.lex_content(substate, r),
				State::Element{ kind, state: substate } => self.lex_element(kind, substate, r),
				State::Reference{ ctx, ret, kind } => self.lex_reference(ctx, ret, kind, r),
				State::CDataSection(nend) => self.lex_cdata_section(nend, r),
				State::Eof => return Ok(None),
			};
			let st = match stresult {
				Err(Error::IO(ioerr)) => {
					// we do not cache I/O errors
					return Err(Error::IO(ioerr));
				},
				Err(other) => {
					// we cache all other errors because we don't want to read / emit invalid data
					let err = other.clone();
					self.err = Some(other);
					return Err(err);
				},
				Ok(st) => st,
			};
			match st.splice(&mut self.state) {
				Some(tok) => {
					#[cfg(debug_assertions)]
					{
						// preserve the state for infinite loop detection
						self.prev_state = (self.scratchpad.clone(), self.state.clone());
					}
					return Ok(Some(tok));
				},
				None => (),
			};
			#[cfg(debug_assertions)]
			{
				// we did not leave the loop; assert that the state has
				// actually changed
				if self.prev_state.0 == self.scratchpad && self.prev_state.1 == self.state {
					panic!("state has not changed in the last iteration: {:?} {:?} last read: {:?}", self, self.scratchpad, self.last_single_read)
				} else {
					self.prev_state = (self.scratchpad.clone(), self.state.clone())
				}
			}
		}
	}
}

impl fmt::Debug for Lexer {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("Lexer")
			.field("state", &self.state)
			.finish()
	}
}

pub trait Sink {
	type ErrorType;

	fn token(&mut self, token: Token);
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::fmt;
	use std::error;
	use crate::lexer::read::DecodingReader;
	use crate::bufq::BufferQueue;

	/// Stream tokens to the sink until the end of stream is reached.
	fn stream_to_sink<'r, 's, 'l, R: CodepointRead, S: Sink>(l: &'l mut Lexer, r: &'r mut R, s: &'s mut S) -> Result<()> {
		loop {
			match l.lex(r) {
				Ok(Some(tok)) => {
					s.token(tok);
				},
				Ok(None) => {
					break;
				},
				Err(e) => return Err(e),
			}
		}
		Ok(())
	}

	fn stream_to_sink_from_bytes<'r, 's, 'l, R: io::BufRead + ?Sized, S: Sink>(l: &'l mut Lexer, r: &'r mut R, s: &'s mut S) -> Result<()> {
		let mut r = DecodingReader::new(r);
		loop {
			match l.lex(&mut r) {
				Ok(Some(tok)) => {
					s.token(tok);
				},
				Ok(None) => {
					break;
				},
				Err(e) => return Err(e),
			}
		}
		Ok(())
	}

	struct VecSink {
		dest: Vec<Token>,
		limit: usize,
	}

	impl VecSink {
		fn new(limit: usize) -> VecSink {
			VecSink {
				dest: Vec::new(),
				limit: limit,
			}
		}
	}

	#[derive(Debug, Clone, PartialEq)]
	struct VecSinkError(String);

	impl fmt::Display for VecSinkError {
		fn fmt<'a>(&self, f: &'a mut fmt::Formatter) -> fmt::Result {
			f.write_str(self.0.as_str())
		}
	}

	impl error::Error for VecSinkError {
		fn source(&self) -> Option<&(dyn error::Error + 'static)> {
			None
		}
	}

	impl Sink for VecSink {
		type ErrorType = io::Error;

		fn token(&mut self, token: Token) {
			if self.dest.len() >= self.limit {
				panic!("token limit exceeded: {}", self.limit);
			}
			self.dest.push(token);
		}
	}

	fn lex(data: &[u8], token_limit: usize) -> (Vec<Token>, Result<()>) {
		let mut buff = io::BufReader::new(data);
		let mut src = DecodingReader::new(&mut buff);
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(token_limit);
		let result = stream_to_sink(&mut lexer, &mut src, &mut sink);
		(sink.dest, result)
	}

	fn lex_err(data: &[u8], token_limit: usize) -> Option<Error> {
		let (_, r) = lex(data, token_limit);
		r.err()
	}

	fn run_fuzz_test(data: &[u8], token_limit: usize) -> Result<Vec<Token>> {
		let mut buff = io::BufReader::new(data);
		let mut src = DecodingReader::new(&mut buff);
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(token_limit);
		stream_to_sink(&mut lexer, &mut src, &mut sink)?;
		Ok(sink.dest)
	}

	#[test]
	fn lexer_lex_xml_decl_start() {
		let mut src = "<?xml".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();

		assert_eq!(sink.dest[0], Token::XMLDeclStart);
	}

	#[test]
	fn lexer_lex_rejects_invalid_xml_decl_opener() {
		let mut src = "<?xmlversion".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		let err = stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();
		assert!(!matches!(err, Error::NotWellFormed(WFError::InvalidEof(..))));

		assert_eq!(sink.dest[0], Token::XMLDeclStart);
		assert_eq!(sink.dest.len(), 1);
	}

	#[test]
	fn lexer_lex_xml_decl_version_name() {
		let mut src = "<?xml version=".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();

		assert_eq!(sink.dest[1], Token::Name(Name::from_str("version").unwrap()));
	}

	#[test]
	fn lexer_lex_xml_decl_version_eq() {
		let mut src = "<?xml version=".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();

		assert_eq!(sink.dest[2], Token::Eq);
	}

	#[test]
	fn lexer_lex_xml_decl_version_value_squot() {
		let mut src = "<?xml version='1.0'".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();

		assert_eq!(sink.dest[3], Token::AttributeValue(CData::from_str("1.0").unwrap()));
	}

	#[test]
	fn lexer_lex_xml_decl_version_value_dquot() {
		let mut src = "<?xml version=\"1.0\"".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();

		assert_eq!(sink.dest[3], Token::AttributeValue(CData::from_str("1.0").unwrap()));
	}

	#[test]
	fn lexer_lex_xml_decl_end() {
		let mut src = "<?xml version=\"1.0\"?>".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		assert_eq!(sink.dest[4], Token::XMLDeclEnd);
	}

	#[test]
	fn lexer_lex_xml_decl_complete() {
		let mut src = "<?xml version=\"1.0\" encoding='utf-8'?>".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink);

		assert!(result.is_ok());
		assert_eq!(sink.dest[0], Token::XMLDeclStart);
		assert_eq!(sink.dest[1], Token::Name(Name::from_str("version").unwrap()));
		assert_eq!(sink.dest[2], Token::Eq);
		assert_eq!(sink.dest[3], Token::AttributeValue(CData::from_str("1.0").unwrap()));
		assert_eq!(sink.dest[4], Token::Name(Name::from_str("encoding").unwrap()));
		assert_eq!(sink.dest[5], Token::Eq);
		assert_eq!(sink.dest[6], Token::AttributeValue(CData::from_str("utf-8").unwrap()));
		assert_eq!(sink.dest[7], Token::XMLDeclEnd);
	}

	#[test]
	fn lexer_lex_element_start() {
		let mut src = &b"<element "[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart(Name::from_str("element").unwrap()));
	}

	#[test]
	fn lexer_lex_element_noattr_empty() {
		let mut src = &b"<element/>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart(Name::from_str("element").unwrap()));
		assert_eq!(sink.dest[1], Token::ElementHeadClose);
	}

	#[test]
	fn lexer_lex_element_noattr_open() {
		let mut src = &b"<element>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart(Name::from_str("element").unwrap()));
		assert_eq!(sink.dest[1], Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_element_noattr_empty_explicit() {
		let mut src = &b"<element></element>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart(Name::from_str("element").unwrap()));
		assert_eq!(sink.dest[1], Token::ElementHFEnd);
		assert_eq!(sink.dest[2], Token::ElementFootStart(Name::from_str("element").unwrap()));
		assert_eq!(sink.dest[3], Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_element_attribute() {
		let mut src = &b"<element x='foo' y=\"bar\" xmlns='baz' xmlns:abc='fnord'>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("element").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Name(Name::from_str("x").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue(CData::from_str("foo").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Name(Name::from_str("y").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue(CData::from_str("bar").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Name(Name::from_str("xmlns").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue(CData::from_str("baz").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Name(Name::from_str("xmlns:abc").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue(CData::from_str("fnord").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_text() {
		let mut src = &b"<root>Hello World!</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text(CData::from_str("Hello World!").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_amp() {
		let mut src = &b"<root>&amp;</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text(CData::from_str("&").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_decimal_charref() {
		let mut src = &b"<root>&#60;</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text(CData::from_str("<").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_hexadecimal_charref() {
		let mut src = &b"<root>&#x3e;</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text(CData::from_str(">").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_mixed_text_entities() {
		let mut src = &b"<root>&#60;example foo=&quot;bar&quot; baz=&apos;fnord&apos;/&gt;</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);

		let mut texts: Vec<String> = Vec::new();
		for tok in iter {
			match tok {
				Token::Text(t) => texts.push(t.to_string()),
				_ => break,
			}
		}

		let text = texts.join("");
		assert_eq!(text.as_str(), "<example foo=\"bar\" baz='fnord'/>");
	}

	#[test]
	fn lexer_lex_reject_charref_with_invalid_cdata() {
		let mut src = &b"<root>&#x00;</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink);
		assert!(matches!(result, Err(Error::NotWellFormed(_))));
	}

	#[test]
	fn lexer_lex_attribute_amp() {
		let mut src = &b"<root foo='&amp;'>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Name(Name::from_str("foo").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue(CData::from_str("&").unwrap()));
	}

	#[test]
	fn lexer_lex_attribute_mixed_with_entities() {
		let mut src = &b"<root foo='&#60;example foo=&quot;bar&quot; baz=&apos;fnord&apos;/&gt;'>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Name(Name::from_str("foo").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue(CData::from_str("<example foo=\"bar\" baz='fnord'/>").unwrap()));
	}

	#[test]
	fn lexer_lex_cdata_section() {
		let mut src = &b"<root><![CDATA[<example foo=\"bar\" baz='fnord'/>]]></root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text(CData::from_str("<example foo=\"bar\" baz='fnord'/>").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_cdata_section_degenerate() {
		let mut src = &b"<root><![CDATA[]]></root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_cdata_section_mixed() {
		let mut src = &b"<root>foobar <![CDATA[Hello <fun>]]</fun>&amp;games world!]]> </root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("root").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);


		let mut texts: Vec<String> = Vec::new();
		for tok in iter {
			match tok {
				Token::Text(t) => texts.push(t.to_string()),
				_ => break,
			}
		}

		let text = texts.join("");
		assert_eq!(text.as_str(), "foobar Hello <fun>]]</fun>&amp;games world! ");
	}

	#[test]
	fn lexer_lex_restrict_element_name_by_token_length() {
		let src = &b"<foobar2342/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(Error::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_restrict_attribute_name_by_token_length() {
		let src = &b"<a foobar2342='foo'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(Error::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_restrict_attribute_value_by_token_length() {
		let src = &b"<a b='foobar2342'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(Error::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_restrict_attribute_value_by_token_length_even_with_entities() {
		let src = &b"<a b='foob&amp;r'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(Error::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_attribute_value_entities_do_only_count_for_expansion() {
		let src = &b"<a b='foob&amp;'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink).unwrap();
	}

	#[test]
	fn lexer_lex_token_length_causes_text_nodes_to_be_split() {
		let src = &b"<a>foo001foo002foo003</a>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("a").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text(CData::from_str("foo001").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Text(CData::from_str("foo002").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Text(CData::from_str("foo003").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart(Name::from_str("a").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_handles_broken_numeric_entity_correctly() {
		// trimmed testcase, found by afl
		let src = &b"&#;"[..];
		let result = run_fuzz_test(src, 128);
		assert!(result.is_err());
	}

	#[test]
	fn lexer_limits_decimal_entities() {
		let src = &b"&#9999999999;"[..];
		let result = run_fuzz_test(src, 128);
		assert!(result.is_err());
	}

	#[test]
	fn lexer_limits_hexadecimal_entities() {
		let src = &b"&#x9999999999;"[..];
		let result = run_fuzz_test(src, 128);
		assert!(result.is_err());
	}

	#[test]
	fn lexer_rejects_invalid_names() {
		let err = lex_err(b"<123/>", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::UnexpectedChar(..))));

		let err = lex_err(b"<'foo/>", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::UnexpectedChar(..))));

		let err = lex_err(b"<.bar/>", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::UnexpectedChar(..))));
	}

	#[test]
	fn lexer_rejects_undeclared_or_invalid_references() {
		let err = lex_err(b"&123;", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::UndeclaredEntity)));

		let err = lex_err(b"&foobar;", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::UndeclaredEntity)));

		let err = lex_err(b"&?;", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::UnexpectedChar(..))));
	}

	#[test]
	fn lexer_rejects_non_scalar_char_refs() {
		let err = lex_err(b"&#x110000;", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::InvalidChar(_, _, true))));
	}

	#[test]
	fn lexer_rejects_non_xml_10_chars_via_refs_in_text() {
		let err = lex_err(b"&#x00;", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::InvalidChar(_, _, true))));

		let err = lex_err(b"&#x1f;", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::InvalidChar(_, _, true))));
	}

	#[test]
	fn lexer_rejects_non_xml_10_chars_via_refs_in_attrs() {
		let err = lex_err(b"<a foo='&#x00;'/>", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::InvalidChar(_, _, true))));

		let err = lex_err(b"<a foo='&#x1f;'/>", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::InvalidChar(_, _, true))));
	}

	#[test]
	fn lexer_rejects_non_xml_10_chars_verbatim_in_text() {
		let err = lex_err(b"\x00", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::InvalidChar(_, _, false))));

		let err = lex_err(b"\x1f", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::InvalidChar(_, _, false))));
	}

	#[test]
	fn lexer_rejects_non_xml_10_chars_verbatim_in_attrs() {
		let err = lex_err(b"<a foo='\x00'/>", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::InvalidChar(_, _, false))));

		let err = lex_err(b"<a foo='\x1f'/>", 128).unwrap();
		assert!(matches!(err, Error::NotWellFormed(WFError::InvalidChar(_, _, false))));
	}

	#[test]
	fn lexer_re_emits_error_on_next_call() {
		let src = &b"<a>\x00</a>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		let e1 = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink).err().unwrap();
		let e2 = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink).err().unwrap();
		assert_eq!(e1, e2);

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("a").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert!(iter.next().is_none());
	}

	#[test]
	fn lexer_handles_closing_brackets_in_cdata_section() {
		let mut src = &b"<a><![CDATA[]]]></a>"[..];
		let mut lexer = Lexer::with_options(LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart(Name::from_str("a").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text(CData::from_str("]").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart(Name::from_str("a").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_recovers_from_wouldblock() {
		let seq = &b"<?xml version='1.0'?>"[..];
		let mut r = DecodingReader::new(BufferQueue::new());
		let mut lexer = Lexer::new();
		let mut sink: Vec<Token> = Vec::new();
		for chunk in seq.chunks(5) {
			r.get_mut().push(std::borrow::Cow::from(chunk));
			loop {
				match lexer.lex(&mut r) {
					Err(Error::IO(ioerr)) if ioerr.kind() == io::ErrorKind::WouldBlock => break,
					Err(other) => panic!("unexpected error: {:?}", other),
					Ok(None) => panic!("unexpected eof signal: {:?}", lexer),
					Ok(Some(tok)) => sink.push(tok),
				}
			}
		}

		let mut iter = sink.iter();
		assert_eq!(*iter.next().unwrap(), Token::XMLDeclStart);
		assert_eq!(*iter.next().unwrap(), Token::Name(Name::from_str("version").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue(CData::from_str("1.0").unwrap()));
		assert_eq!(*iter.next().unwrap(), Token::XMLDeclEnd);
	}
}
