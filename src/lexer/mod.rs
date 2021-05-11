use std::io;
use std::fmt;

mod selectors;
mod read;

use selectors::*;
use read::{CodepointRead, Utf8Char, read_validated, CharSelector, Endpoint, skip_matching};
use crate::error::{Error, Result, WFError};

pub use read::DecodingReader;

const ERRCTX_UNKNOWN: &'static str = "in unknown context";
const ERRCTX_TEXT: &'static str = "in text node";
const ERRCTX_ATTVAL: &'static str = "in attribute value";
const ERRCTX_NAME: &'static str = "in name";
const ERRCTX_NAMESTART: &'static str = "at start of name";
const ERRCTX_ELEMENT: &'static str = "in element";
const ERRCTX_ELEMENT_FOOT: &'static str = "in element footer";
const ERRCTX_ELEMENT_CLOSE: &'static str = "at element close";
const ERRCTX_CDATA_SECTION: &'static str = "in CDATA section";
const ERRCTX_CDATA_SECTION_START: &'static str = "at CDATA section marker";
const ERRCTX_XML_DECL: &'static str = "in XML declaration";
const ERRCTX_XML_DECL_START: &'static str = "at start of XML declaration";
const ERRCTX_XML_DECL_END: &'static str = "at end of XML declaration";
const ERRCTX_REF: &'static str = "in entity or character reference";

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
	/* the following tokens are only emitted in the Element lexer state */
	Name(String),
	Eq,  // =
	AttributeValue(String),  // '...' | "..."
	XMLDeclEnd,  // ?>
	ElementHeadClose,  // />
	ElementHFEnd,  // >

	/* the following tokens are only emitted in the Content lexer state */
	Reference(char), // &...;
	XMLDeclStart,  // <?xml
	ElementHeadStart(String),  // <
	ElementFootStart(String),  // </

	/* the following tokens may be emitted in the CDataSection and Content
	lexer states */
	Text(String),
}

#[derive(Debug, Clone, PartialEq)]
enum CharRefRadix {
	Decimal,
	Hexadecimal,
}

#[derive(Debug, Clone, PartialEq)]
enum RefKind {
	Entity,
	Char(CharRefRadix),
}

#[derive(Debug, Clone, PartialEq)]
enum ElementState {
	Start,
	Blank,
	Name,
	Eq,
	Close,
	/// Delimiter and Alphabet
	AttributeValue(char, CodepointRanges<'static>),
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

#[derive(Debug, Clone, PartialEq)]
enum MaybeElementState {
	Initial,
	/// Number of correct CDATA section start characters
	CDataSectionStart(usize),
	/// Number of correct XML decl start characters
	XMLDeclStart(usize),
}

#[derive(Debug, Clone, PartialEq)]
enum ContentState {
	Initial,
	/// Encountered <
	MaybeElement(MaybeElementState),
}

#[derive(Debug, Clone, PartialEq)]
enum State {
	Content(ContentState),
	Element{ kind: ElementKind, state: ElementState },

	/// encountered &
	Reference{ ctx: &'static str, ret: Box<State>, kind: RefKind },

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

const MAX_REFERENCE_LENGTH: usize = 32usize;

const TOK_XML_DECL_START: &'static [u8] = b"<?xml";
const TOK_XML_CDATA_START: &'static [u8] = b"<![CDATA[";
// const CLASS_XML_NAME_START_CHAR:

#[derive(Debug, Clone, PartialEq, Copy)]
pub struct LexerOptions {
	pub max_token_length: usize,
}

impl LexerOptions {
	pub fn defaults() -> LexerOptions {
		LexerOptions{
			max_token_length: 65535,
		}
	}

	pub fn max_token_length<'a>(&'a mut self, v: usize) -> &'a mut LexerOptions {
		self.max_token_length = v;
		self
	}
}

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
}

struct ST(State, Option<Token>);

impl ST {
	fn splice<'a>(self, st: &'a mut State) -> Option<Token> {
		*st = self.0;
		self.1
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
		Err(Error::NotWellFormed(WFError::UnexpectedChar(ERRCTX_UNKNOWN, ch, None)))
	}
}

fn add_context<T>(r: Result<T>, ctx: &'static str) -> Result<T> {
	r.or_else(|e| { match e {
		Error::NotWellFormed(wf) => Err(Error::NotWellFormed(wf.with_context(ctx))),
		other => Err(other),
	} })
}

fn handle_eof<T>(v: Option<T>, ctx: &'static str) -> Result<T> {
	v.ok_or_else(|| {
		Error::wfeof(ctx)
	})
}

impl Lexer {
	pub fn new() -> Lexer {
		Lexer::with_options(LexerOptions::defaults())
	}

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


	/// Skip all spaces and *peek* at the byte after that.
	/* fn skip_spaces<'r, R: io::BufRead + ?Sized>(&mut self, r: &'r mut R) -> io::Result<u8> {
		let selector = InvertDelimiters(CLASS_XML_SPACES);
		match selector.find_any_first_in(self.buf.as_slice()) {
			Some((sz, delim)) => {
				if sz > 0 {
					self.buf.drain(..sz-1);
				}
				return Ok(self.buf[0]);
			},
			None => {
				self.buf.clear();
			},
		};
		discard_up_to(r, &selector)
	}

	fn read_single<'r, R: io::BufRead + ?Sized>(&mut self, r: &'r mut R) -> io::Result<u8> {
		if self.buf.len() > 0 {
			Ok(self.buf.remove(0))
		} else {
			let mut result = 0u8;
			r.read_exact(slice::from_mut(&mut result))?;
			Ok(result)
		}
	}

	fn peek_single<'r, R: io::BufRead + ?Sized>(&mut self, r: &'r mut R) -> io::Result<u8> {
		if self.buf.len() > 0 {
			Ok(self.buf[0])
		} else {
			let mut result = 0u8;
			r.read_exact(slice::from_mut(&mut result))?;
			self.buf.push(result);
			Ok(result)
		}
	}

	fn flush_to_scratchpad(&mut self, data: Vec<u8>) -> Result<()> {
		let add_size = data.len();
		if add_size == 0 {
			return Ok(())
		}
		let new_size = match self.scratchpad.len().checked_add(add_size) {
			None => return Err(Error::RestrictedXml(format!("length overflow"))),
			Some(sz) => sz,
		};
		if new_size >= self.opts.max_token_length {
			return Err(self.token_length_error());
		}
		self.scratchpad.push_str(String::from_utf8(data)?.as_str());
		Ok(())
	}

	fn read_scratchpad_unshrunk(&mut self) -> Result<String> {
		let mut result = {
			let mut s = String::new();
			std::mem::swap(&mut s, &mut self.scratchpad);
			s
		};
		Ok(result)
	}

	fn read_scratchpad(&mut self) -> Result<String> {
		let mut result = self.read_scratchpad_unshrunk()?;
		result.shrink_to_fit();
		Ok(result)
	}

	fn read_scratchpad_with(&mut self, more: String) -> Result<String> {
		if self.scratchpad.len() == 0 {
			return Ok(more)
		}
		let mut result = self.read_scratchpad_unshrunk()?;
		result.push_str(more.as_str());
		result.shrink_to_fit();
		Ok(result)
	}

	fn flush_text_safe(&mut self, data: Vec<u8>) -> Result<Option<Token>> {
		let text = self.read_scratchpad_with(String::from_utf8(data)?)?;
		if text.len() == 0 {
			return Ok(None)
		}
		Ok(Some(Token::Text(text)))
	} */

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

	fn maybe_flush_scratchpad_as_text(&mut self) -> Result<Option<Token>> {
		if self.scratchpad.len() == 0 {
			Ok(None)
		} else {
			Ok(Some(Token::Text(self.flush_scratchpad()?)))
		}
	}

	fn lex_content<'r, R: CodepointRead>(&mut self, state: ContentState, r: &'r mut R) -> Result<ST>
	{
		println!("{:?}", state);
		match state {
			// read until next `<` or `&`, which are the only things which
			// can break us out of this state.
			ContentState::Initial => match self.read_validated(r, &CodepointRanges(VALID_XML_CDATA_RANGES_TEXT_DELIMITED), self.opts.max_token_length)? {
				Endpoint::Eof => {
					println!("eof");
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
								ret: Box::new(State::Content(ContentState::Initial)),
								kind: RefKind::Entity,
							},
							tok,
						))
					},
					other => Err(Error::NotWellFormed(WFError::UnexpectedChar(ERRCTX_TEXT, other, None))),
				},
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
							state: ElementState::Blank,
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
							ElementKind::Header => Token::ElementHeadStart(self.flush_scratchpad()?),
							ElementKind::Footer => Token::ElementFootStart(self.flush_scratchpad()?),
							ElementKind::XMLDecl => panic!("invalid state"),
						}),
					))
				},
			},
			// look for next non-space thing and check what to do with it
			ElementState::Blank => match skip_matching(r, &CLASS_XML_SPACES)? {
				Endpoint::Eof | Endpoint::Limit => Err(Error::wfeof(ERRCTX_ELEMENT)),
				Endpoint::Delimiter(ch) => Ok(ST(
					State::Element{
						kind: kind,
						state: self.lex_element_postblank(kind, ch)?,
					},
					None,
				)),
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
						Some(Token::Name(self.flush_scratchpad()?)),
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
								ret: Box::new(State::Element{
									kind: kind,
									state: ElementState::AttributeValue(delim, selector),
								}),
								kind: RefKind::Entity,
							}, None
						))
					},
					d if d == delim => Ok(ST(
						State::Element{
							kind: kind,
							state: ElementState::Blank
						},
						Some(Token::AttributeValue(self.flush_scratchpad()?)),
					)),
					other => Err(Error::NotWellFormed(WFError::UnexpectedChar(
						ERRCTX_ATTVAL,
						other,
						None,
					)))
				},
			},
			ElementState::MaybeXMLDeclEnd => match self.read_single(r)? {
				Some(ch) if ch.to_char() == '>' => {
					self.drop_scratchpad()?;
					Ok(ST(
						State::Content(ContentState::Initial),
						Some(Token::XMLDeclEnd),
					))
				},
				Some(ch) => Err(Error::NotWellFormed(WFError::UnexpectedChar(
					ERRCTX_XML_DECL_END,
					ch.to_char(),
					Some(&[">"]),
				))),
				None => panic!("eof during xml decl"),
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
				None => panic!("eof during element"),
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

	fn lex_reference<'r, R: CodepointRead>(&mut self, ctx: &'static str, ret: Box<State>, kind: RefKind, r: &'r mut R) -> Result<ST> {
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
			Ok(_) => Ok(ST(*ret, None)),
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
				if ch == '>' {
					// end of CDATA section, flush the scratchpad
					Ok(ST(
						State::Content(ContentState::Initial),
						self.maybe_flush_scratchpad_as_text()?,
					))
				} else {
					// not a '>', thus we have to add the two ']' and whatever we just found to the scratchpad
					self.scratchpad.push_str("]]");
					self.scratchpad.push_str(utf8ch.as_str());
					// continue with the CDATA section as before without unnecessary flush
					Ok(ST(
						State::CDataSection(0),
						None,
					))
				}
			},
			_ => panic!("invalid state"),
		}
	}

	fn is_valid_terminal_state(&self) -> bool {
		match &self.state {
			State::Content(ContentState::Initial) => true,
			_ => false,
		}
	}

	/// Lex bytes from the reader until either an error occurs, a valid
	/// token is produced or the stream ends between two tokens.
	///
	/// **Note**: While it is possible to swap the reader, that is highly
	/// not recommended as the lexer state may depend on lookaheads.
	pub fn lex<'r, R: CodepointRead>(&mut self, r: &'r mut R) -> Result<Option<Token>>
	{
		loop {
			let result = match self.state.clone() {
				State::Content(substate) => self.lex_content(substate.clone(), r),
				State::Element{ kind, state: substate } => self.lex_element(kind, substate.clone(), r),
				State::Reference{ ctx, ret, kind } => self.lex_reference(ctx, ret, kind, r),
				State::CDataSection(nend) => self.lex_cdata_section(nend, r),
				State::Eof => return Ok(None),
			};
			let st = match result {
				Err(e) => match e {
					Error::IO(ref sube) if sube.kind() == io::ErrorKind::UnexpectedEof => {
						if self.is_valid_terminal_state() {
							// Important to return here to break out of the loop.
							return Ok(None)
						} else {
							Err(e)
						}
					},
					e => Err(e),
				},
				Ok(st) => Ok(st),
			}?;
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
	fn lexer_lex_xml_decl_version_name() {
		let mut src = "<?xml version=".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();

		assert_eq!(sink.dest[1], Token::Name("version".to_string()));
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

		assert_eq!(sink.dest[3], Token::AttributeValue("1.0".to_string()));
	}

	#[test]
	fn lexer_lex_xml_decl_version_value_dquot() {
		let mut src = "<?xml version=\"1.0\"".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();

		assert_eq!(sink.dest[3], Token::AttributeValue("1.0".to_string()));
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
		assert_eq!(sink.dest[1], Token::Name("version".to_string()));
		assert_eq!(sink.dest[2], Token::Eq);
		assert_eq!(sink.dest[3], Token::AttributeValue("1.0".to_string()));
		assert_eq!(sink.dest[4], Token::Name("encoding".to_string()));
		assert_eq!(sink.dest[5], Token::Eq);
		assert_eq!(sink.dest[6], Token::AttributeValue("utf-8".to_string()));
		assert_eq!(sink.dest[7], Token::XMLDeclEnd);
	}

	#[test]
	fn lexer_lex_element_start() {
		let mut src = &b"<element "[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).err().unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart("element".to_string()));
	}

	#[test]
	fn lexer_lex_element_noattr_empty() {
		let mut src = &b"<element/>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart("element".to_string()));
		assert_eq!(sink.dest[1], Token::ElementHeadClose);
	}

	#[test]
	fn lexer_lex_element_noattr_open() {
		let mut src = &b"<element>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart("element".to_string()));
		assert_eq!(sink.dest[1], Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_element_noattr_empty_explicit() {
		let mut src = &b"<element></element>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart("element".to_string()));
		assert_eq!(sink.dest[1], Token::ElementHFEnd);
		assert_eq!(sink.dest[2], Token::ElementFootStart("element".to_string()));
		assert_eq!(sink.dest[3], Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_element_attribute() {
		let mut src = &b"<element x='foo' y=\"bar\" xmlns='baz' xmlns:abc='fnord'>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("element".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Name("x".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue("foo".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Name("y".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue("bar".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Name("xmlns".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue("baz".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Name("xmlns:abc".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue("fnord".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_text() {
		let mut src = &b"<root>Hello World!</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text("Hello World!".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_amp() {
		let mut src = &b"<root>&amp;</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text("&".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_decimal_charref() {
		let mut src = &b"<root>&#60;</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text("<".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_hexadecimal_charref() {
		let mut src = &b"<root>&#x3e;</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text(">".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_mixed_text_entities() {
		let mut src = &b"<root>&#60;example foo=&quot;bar&quot; baz=&apos;fnord&apos;/&gt;</root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);

		let mut texts: Vec<String> = Vec::new();
		for tok in iter {
			match tok {
				Token::Text(t) => texts.push(t.clone()),
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
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Name("foo".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue("&".to_string()));
	}

	#[test]
	fn lexer_lex_attribute_mixed_with_entities() {
		let mut src = &b"<root foo='&#60;example foo=&quot;bar&quot; baz=&apos;fnord&apos;/&gt;'>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Name("foo".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Eq);
		assert_eq!(*iter.next().unwrap(), Token::AttributeValue("<example foo=\"bar\" baz='fnord'/>".to_string()));
	}

	#[test]
	fn lexer_lex_cdata_section() {
		let mut src = &b"<root><![CDATA[<example foo=\"bar\" baz='fnord'/>]]></root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text("<example foo=\"bar\" baz='fnord'/>".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_cdata_section_degenerate() {
		let mut src = &b"<root><![CDATA[]]></root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_cdata_section_mixed() {
		let mut src = &b"<root>foobar <![CDATA[Hello <fun>]]</fun>&amp;games world!]]> </root>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut src, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("root".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);


		let mut texts: Vec<String> = Vec::new();
		for tok in iter {
			match tok {
				Token::Text(t) => texts.push(t.clone()),
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
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(Error::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_restrict_attribute_name_by_token_length() {
		let src = &b"<a foobar2342='foo'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(Error::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_restrict_attribute_value_by_token_length() {
		let src = &b"<a b='foobar2342'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(Error::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_restrict_attribute_value_by_token_length_even_with_entities() {
		let src = &b"<a b='foob&amp;r'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(Error::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_attribute_value_entities_do_only_count_for_expansion() {
		let src = &b"<a b='foob&amp;'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink).unwrap();
	}

	#[test]
	fn lexer_lex_token_length_causes_text_nodes_to_be_split() {
		let src = &b"<a>foo001foo002foo003</a>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		stream_to_sink_from_bytes(&mut lexer, &mut buffered, &mut sink).unwrap();

		let mut iter = sink.dest.iter();
		assert_eq!(*iter.next().unwrap(), Token::ElementHeadStart("a".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
		assert_eq!(*iter.next().unwrap(), Token::Text("foo001".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Text("foo002".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::Text("foo003".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementFootStart("a".to_string()));
		assert_eq!(*iter.next().unwrap(), Token::ElementHFEnd);
	}

	#[test]
	fn fuzz_9b174f7ff3245b18() {
		let src = &b"<!xml v<!xml v\x85rers\x8bon<!xml \x03\xe8\xff"[..];
		let result = run_fuzz_test(src, 128);
		assert!(result.is_err());
		assert!(matches!(result, Err(Error::NotWellFormed(_))));
	}

	#[test]
	fn fuzz_35cabf8da64df7d1() {
		let src = &b"\x10\x00<!"[..];
		let result = run_fuzz_test(src, 128);
		assert!(result.is_err());
	}

	#[test]
	fn fuzz_9bde23591fb17cd7() {
		let src = &b"<\x01\x00m\x00\x00\x02\x00\x00?xml\x20vkrsl\x20<?xml\x20vkrs\x00\x30\x27\x00?>\x0a"[..];
		let result = run_fuzz_test(src, 128);
		assert!(result.is_err());
	}
}
