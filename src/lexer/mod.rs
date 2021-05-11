use std::io;
use std::mem;
use std::error;
use std::string;
use std::fmt;
use std::slice;
use std::result::Result as StdResult;

mod selectors;
mod utf8;
mod read;

use selectors::*;
use utf8::*;

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
	Blank,
	Name,
	Eq,
	Close,
	/// Delimiter
	AttributeValue(u8),
	/// Encountered ?
	MaybeXMLDeclEnd,
	/// Encountered /
	MaybeHeadClose,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ElementKind {
	/// standard XML element head e.g. `<foo>`
	ElementHead,
	/// standard XML element foot e.g. `</foo>`
	ElementFoot,
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
	/// Closing element
	Footer,
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
	Reference{ ret: Box<State>, kind: RefKind },

	/// Count the number of correct consecutive CDataSectionEnd characters
	/// encountered
	CDataSection(usize),
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
	buf: Vec<u8>,
	scratchpad: String,
	opts: LexerOptions,
}

fn read_up_to_limited<'x, R: io::BufRead + ?Sized, S: ByteSelector<'x>>(r: &mut R, delimiters: &'x S, buf: &mut Vec<u8>, limit: usize) -> io::Result<(usize, Option<u8>)> {
	let mut nread = 0;
	loop {
		let (done, used) = {
			let available = match r.fill_buf() {
				Ok(b) => b,
				Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
				Err(e) => return Err(e),
			};
			if available.len() == 0 {
				// EOF!
				return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof while scanning for delimiter"));
			}
			match delimiters.find_any_first_in(available) {
				Some((offset, matched)) => {
					// Do not increase offset by one, we don’t want to discard
					// the next match.
					buf.extend_from_slice(&available[..offset]);
					(Some(matched), offset)
				},
				None => {
					buf.extend_from_slice(&available);
					(None, available.len())
				},
			}
		};
		r.consume(used);
		nread += used;
		if let Some(matched) = done {
			return Ok((nread, Some(matched)))
		}
		if nread >= limit {
			return Ok((nread, None))
		}
	}
}

fn discard_up_to<'x, R: io::BufRead + ?Sized, S: ByteSelector<'x>>(r: &mut R, delimiters: &'x S) -> io::Result<u8> {
	loop {
		let (done, used) = {
			let available = match r.fill_buf() {
				Ok(b) => b,
				Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
				Err(e) => return Err(e),
			};
			if available.len() == 0 {
				// EOF!
				return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof while scanning for delimiter"));
			}
			match delimiters.find_any_first_in(available) {
				// Do not increase offset by one, we don’t want to discard the
				// next match.
				Some((offset, matched)) => (Some(matched), offset),
				None => (None, available.len()),
			}
		};
		r.consume(used);
		if let Some(matched) = done {
			return Ok(matched)
		}
	}
}

#[derive(Debug)]
pub enum LexerError {
	IO(io::Error),
	Utf8(string::FromUtf8Error),
	NotWellFormed(String),
	NotValid,
	RestrictedXml(String),
}

type Result<T> = StdResult<T, LexerError>;

impl LexerError {
	fn io(e: io::Error) -> LexerError {
		LexerError::IO(e)
	}

	fn utf8(e: string::FromUtf8Error) -> LexerError {
		LexerError::Utf8(e)
	}
}

impl From<io::Error> for LexerError {
	fn from(e: io::Error) -> LexerError {
		LexerError::io(e)
	}
}

impl From<string::FromUtf8Error> for LexerError {
	fn from(e: string::FromUtf8Error) -> LexerError {
		LexerError::utf8(e)
	}
}

impl fmt::Display for LexerError {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		match self {
			LexerError::NotWellFormed(msg) => write!(f, "not well formed: {}", msg),
			LexerError::NotValid => f.write_str("invalid xml"),
			LexerError::RestrictedXml(msg) => write!(f, "restricted xml: {}", msg),
			LexerError::IO(e) => write!(f, "I/O error: {}", e),
			LexerError::Utf8(e) => write!(f, "utf8 error: {}", e),
		}
	}
}

impl error::Error for LexerError {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		match self {
			LexerError::IO(e) => Some(e),
			LexerError::Utf8(e) => Some(e),
			LexerError::NotWellFormed(_) | LexerError::RestrictedXml(_) | LexerError::NotValid => None,
		}
	}
}

struct ST(State, Option<Token>);

impl ST {
	fn splice<'a>(self, st: &'a mut State) -> Option<Token> {
		*st = self.0;
		self.1
	}
}

fn resolve_named_entity(name: &[u8]) -> Result<char> {
	// amp, lt, gt, apos, quot
	match name {
		b"amp" => Ok('&'),
		b"lt" => Ok('<'),
		b"gt" => Ok('>'),
		b"apos" => Ok('\''),
		b"quot" => Ok('"'),
		_ => Err(LexerError::NotWellFormed(format!("undeclared entity: {:?}", DebugBytes(name)))),
	}
}

fn resolve_char_reference(s: &[u8], radix: CharRefRadix) -> Result<char> {
	let s = std::str::from_utf8(s).unwrap();
	let radix = match radix {
		CharRefRadix::Decimal => 10,
		CharRefRadix::Hexadecimal => 16,
	};
	let codepoint = match u32::from_str_radix(s, radix) {
		Ok(v) => v,
		Err(e) => return Err(LexerError::NotWellFormed(format!("invalid codepoint integer: {}", e))),
	};
	let ch = match std::char::from_u32(codepoint) {
		Some(ch) => ch,
		None => return Err(LexerError::NotWellFormed(format!("character reference {:x} expands to invalid char", codepoint))),
	};
	if contained_in_ranges(ch, VALID_XML_CDATA_RANGES) {
		Ok(ch)
	} else {
		Err(LexerError::NotWellFormed(format!("character {:?} is forbidden", ch)))
	}
}

impl Lexer {
	pub fn new() -> Lexer {
		Lexer::with_options(LexerOptions::defaults())
	}

	pub fn with_options(opts: LexerOptions) -> Lexer {
		Lexer {
			state: State::Content(ContentState::Initial),
			buf: Vec::new(),
			scratchpad: String::new(),
			opts: opts,
		}
	}

	fn token_length_error(&self) -> LexerError {
		LexerError::RestrictedXml(format!("maximum token length exceeded"))
	}

	fn read_up_to_limited<'r, 'x, R: io::BufRead + ?Sized, S: ByteSelector<'x>>(&mut self, r: &'r mut R, delimiters: &'x S, limit: usize) -> io::Result<Option<(Vec<u8>, u8)>> {
		match delimiters.find_any_first_in(self.buf.as_slice()) {
			Some((sz, delim)) => {
				let result_vec = self.buf.split_off(sz);
				return Ok(Some((result_vec, delim)));
			},
			None => (),
		};
		if self.buf.len() >= limit {
			// delimiter not found within limit
			return Ok(None)
		}
		let remaining = limit - self.buf.len();
		match read_up_to_limited(r, delimiters, &mut self.buf, remaining)? {
			(added, Some(delim)) => {
				// after read_up_to_limited, the buffer contains the data
				// up to but excluding the delimiter
				let mut result_vec = Vec::new();
				mem::swap(&mut result_vec, &mut self.buf);
				Ok(Some((result_vec, delim)))
			},
			(_, None) => Ok(None),
		}
	}

	/// Skip all spaces and *peek* at the byte after that.
	fn skip_spaces<'r, R: io::BufRead + ?Sized>(&mut self, r: &'r mut R) -> io::Result<u8> {
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
			None => return Err(LexerError::RestrictedXml(format!("length overflow"))),
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
	}

	fn lex_content<'r, R: io::BufRead + ?Sized>(&mut self, state: ContentState, r: &'r mut R) -> Result<ST>
	{
		match state {
			// read until next `<` or `&`, which are the only things which
			// can break us out of this state.
			ContentState::Initial => match self.read_up_to_limited(r, &DELIM_TEXT_STATE_EXIT, self.opts.max_token_length)? {
				Some((data, b'<')) => {
					// guaranteed to succeed because read_up_to_limited is a peek
					self.read_single(r).unwrap();
					Ok(ST(State::Content(ContentState::MaybeElement(MaybeElementState::Initial)), self.flush_text_safe(data)?))
				},
				Some((data, b'&')) => {
					self.read_single(r).unwrap();
					Ok(ST(State::Reference{
						ret: Box::new(State::Content(ContentState::Initial)),
						kind: RefKind::Entity,
					}, self.flush_text_safe(data)?))
				},
				Some((_, other)) => panic!("other {:?}", other),
				None => panic!("looong text node"),
			},
			ContentState::MaybeElement(MaybeElementState::Initial) => match self.peek_single(r)? {
				b'?' => Ok(ST(State::Content(ContentState::MaybeElement(MaybeElementState::XMLDeclStart(1))), None)),
				b'!' => Ok(ST(State::Content(ContentState::MaybeElement(MaybeElementState::CDataSectionStart(1))), None)),
				b'/' => {
					// need to drop the slash so that the name can be read next.
					self.read_single(r)?;
					Ok(ST(State::Content(ContentState::MaybeElement(MaybeElementState::Footer)), None))
				},
				b => {
					if byte_maybe_xml_namestart(b) {
						match self.read_up_to_limited(r, &XMLNonNameBytePreselector(), self.opts.max_token_length)? {
							Some((data, _)) => Ok(ST(State::Element{ kind: ElementKind::ElementHead, state: ElementState::Blank }, Some(Token::ElementHeadStart(String::from_utf8(data)?)))),
							None => Err(self.token_length_error()),
						}
					} else {
						Err(LexerError::NotWellFormed(format!("'<' followed by unexpected byte: {:?}", DebugByte(b))))
					}
				}
			},
			ContentState::MaybeElement(MaybeElementState::XMLDeclStart(i)) => {
				debug_assert!(i < TOK_XML_DECL_START.len());
				let next = self.read_single(r)?;
				if next != TOK_XML_DECL_START[i] {
					return Err(LexerError::RestrictedXml(format!("processing instructions prohibited")));
				}
				let next = i + 1;
				if next == TOK_XML_DECL_START.len() {
					Ok(ST(State::Element{ kind: ElementKind::XMLDecl, state: ElementState::Blank }, Some(Token::XMLDeclStart)))
				} else {
					Ok(ST(State::Content(ContentState::MaybeElement(MaybeElementState::XMLDeclStart(next))), None))
				}
			},
			ContentState::MaybeElement(MaybeElementState::CDataSectionStart(i)) => {
				debug_assert!(i < TOK_XML_CDATA_START.len());
				let next = self.read_single(r)?;
				if i == 1 && next == b'-' {
					return Err(LexerError::RestrictedXml(format!("comments prohibited")));
				} else if next != TOK_XML_CDATA_START[i] {
					return Err(LexerError::NotWellFormed(format!("malformed cdata section start")));
				}
				let next = i + 1;
				if next == TOK_XML_CDATA_START.len() {
					Ok(ST(State::CDataSection(0), self.flush_text_safe(Vec::new())?))
				} else {
					Ok(ST(State::Content(ContentState::MaybeElement(MaybeElementState::CDataSectionStart(next))), None))
				}
			},
			ContentState::MaybeElement(MaybeElementState::Footer) => {
				let next = self.peek_single(r)?;
				if byte_maybe_xml_namestart(next) {
					match self.read_up_to_limited(r, &XMLNonNameBytePreselector(), self.opts.max_token_length)? {
						Some((data, _)) => Ok(ST(State::Element{ kind: ElementKind::ElementFoot, state: ElementState::Blank }, Some(Token::ElementFootStart(String::from_utf8(data)?)))),
						None => panic!("looong element"),
					}
				} else {
					Err(LexerError::NotWellFormed(format!("'</' followed by unexpected byte: {}", next)))
				}
			},
		}
	}

	fn lex_element_postblank<'r, R: io::BufRead + ?Sized>(&mut self, r: &'r mut R, kind: ElementKind, v: u8) -> Result<ElementState> {
		match v {
			b'"' | b'\'' => {
				self.read_single(r)?;
				Ok(ElementState::AttributeValue(v))
			},
			b'=' => {
				Ok(ElementState::Eq)
			},
			v if byte_maybe_xml_name(v) => {
				Ok(ElementState::Name)
			},
			b'?' => match kind {
				ElementKind::XMLDecl => {
					self.read_single(r)?;
					Ok(ElementState::MaybeXMLDeclEnd)
				},
				_ => Err(LexerError::NotWellFormed(format!("'?' not allowed in elements'"))),
			},
			// we could skip here, but that would just be a move of the entire
			// buffer for no real gain.
			b' ' => Ok(ElementState::Blank),
			b'/' => match kind {
				ElementKind::ElementHead => {
					self.read_single(r)?;
					Ok(ElementState::MaybeHeadClose)
				},
				_ => Err(LexerError::NotWellFormed(format!("'/' not allowed in xml declarations or element footers")))
			},
			b'>' => Ok(ElementState::Close),
			_ => Err(LexerError::NotWellFormed(format!("invalid byte in element: {}", v)))
		}
	}

	fn lex_element<'r, R: io::BufRead + ?Sized>(&mut self, kind: ElementKind, state: ElementState, r: &'r mut R) -> Result<ST> {
		match state {
			// look for next non-space thing and check what to do with it
			ElementState::Blank => {
				let v = self.skip_spaces(r)?;
				Ok(ST(State::Element{ kind: kind, state: self.lex_element_postblank(r, kind, v)? }, None))
			},
			ElementState::Name => match self.read_up_to_limited(r, &XMLNonNameBytePreselector(), self.opts.max_token_length)? {
				Some((data, delim)) => {
					Ok(ST(State::Element{ kind: kind, state: self.lex_element_postblank(r, kind, delim)? }, Some(Token::Name(String::from_utf8(data)?))))
				},
				None => Err(self.token_length_error()),
			},
			ElementState::Eq => {
				let next = self.read_single(r)?;
				match next {
					b'=' => Ok(ST(State::Element{ kind: kind, state: ElementState::Blank }, Some(Token::Eq))),
					_ => Err(LexerError::NotWellFormed("expected '='".to_string())),
				}
			},
			ElementState::AttributeValue(delim) => {
				// XML 1.0 §2.3 [10] AttValue
				let delimiters = &[b'<', b'&', delim][..];
				match self.read_up_to_limited(r, &delimiters, self.opts.max_token_length)? {
					Some((data, b'<')) => Err(LexerError::NotWellFormed("'<' encountered in attribute value".to_string())),
					Some((data, b'&')) => {
						self.flush_to_scratchpad(data)?;
						self.read_single(r)?;
						Ok(ST(State::Reference{
							ret: Box::new(State::Element{
								kind: kind,
								state: ElementState::AttributeValue(delim),
							}),
							kind: RefKind::Entity,
						}, None))
					},
					Some((data, delim)) => {
						// end of attribute \o/
						self.flush_to_scratchpad(data)?;
						self.read_single(r)?;
						Ok(ST(State::Element{ kind: kind, state: ElementState::Blank }, Some(Token::AttributeValue(self.read_scratchpad()?))))
					},
					None => Err(self.token_length_error()),
				}
			},
			ElementState::MaybeXMLDeclEnd => {
				let next = self.read_single(r)?;
				match next {
					b'>' => Ok(ST(State::Content(ContentState::Initial), Some(Token::XMLDeclEnd))),
					_ => Err(LexerError::NotWellFormed("'?' not followed by '>' at end of xml declaration".to_string()))
				}
			},
			ElementState::MaybeHeadClose => {
				let next = self.read_single(r)?;
				match next {
					b'>' => Ok(ST(State::Content(ContentState::Initial), Some(Token::ElementHeadClose))),
					_ => Err(LexerError::NotWellFormed("'/' not followed by '>' at end of element".to_string()))
				}
			},
			ElementState::Close => {
				let next = self.read_single(r)?;
				match next {
					b'>' => Ok(ST(State::Content(ContentState::Initial), Some(Token::ElementHFEnd))),
					_ => Err(LexerError::NotWellFormed("'>' expected".to_string()))
				}
			},
		}
	}

	fn lex_reference<'r, R: io::BufRead + ?Sized>(&mut self, ret: Box<State>, kind: RefKind, r: &'r mut R) -> Result<ST> {
		let result = match kind {
			RefKind::Entity => self.read_up_to_limited(r, &XMLNonNameBytePreselector(), MAX_REFERENCE_LENGTH)?,
			RefKind::Char(CharRefRadix::Decimal) => self.read_up_to_limited(r, &InvertDelimiters(&CLASS_XML_DECIMAL_DIGITS), MAX_REFERENCE_LENGTH)?,
			RefKind::Char(CharRefRadix::Hexadecimal) => self.read_up_to_limited(r, &InvertDelimiters(&CLASS_XML_HEXADECIMAL_DIGITS), MAX_REFERENCE_LENGTH)?,
		};
		let result = match result {
			Some((data, b'#')) => {
				if data.len() > 0 {
					Err(b'#')
				} else {
					match kind {
						RefKind::Entity => {
							self.read_single(r)?;
							// early out here because we need to switch states
							return Ok(ST(State::Reference{ ret: ret, kind: RefKind::Char(CharRefRadix::Decimal) }, None))
						},
						_ => Err(b'#'),
					}
				}
			},
			Some((data, b'x')) => {
				if data.len() > 0 {
					Err(b'x')
				} else {
					match kind {
						RefKind::Char(CharRefRadix::Decimal) => {
							self.read_single(r)?;
							// early out here because we need to switch states
							return Ok(ST(State::Reference{ ret: ret, kind: RefKind::Char(CharRefRadix::Hexadecimal) }, None))
						},
						_ => Err(b'x'),
					}
				}
			},
			Some((data, b';')) => {
				self.read_single(r).unwrap();
				match kind {
					RefKind::Entity => Ok(resolve_named_entity(data.as_slice())?),
					RefKind::Char(radix) => Ok(resolve_char_reference(data.as_slice(), radix)?),
				}
			},
			Some((_, invalid)) => Err(invalid),
			None => return Err(LexerError::NotWellFormed(format!("undeclared entity (entity name too long)"))),
		};
		match result {
			Ok(ch) => {
				self.scratchpad.push(ch);
				Ok(ST(*ret, None))
			},
			Err(b) => return Err(LexerError::NotWellFormed(format!("entity must be terminated with ';', not {:?}", DebugByte(b)))),
		}
	}

	fn lex_cdata_section<'r, R: io::BufRead + ?Sized>(&mut self, mut nend: usize, r: &'r mut R) -> Result<ST> {
		if nend == 2 {
			// we read two consecutive b']', so we now need to check for a
			// single b'>'.
			let next = self.read_single(r)?;
			if next == b'>' {
				// end of CDATA section, flush the scratchpad
				return Ok(ST(State::Content(ContentState::Initial), self.flush_text_safe(Vec::new())?))
			} else {
				// not a b'>', thus we have to add the two b']' and whatever
				// we just found to the scratchpad
				self.scratchpad.push_str("]]");
				self.scratchpad.push(next as char);
				// continue with the CDATA section as before
				return Ok(ST(State::CDataSection(0), self.flush_text_safe(Vec::new())?))
			}
		}

		match self.read_up_to_limited(r, &b']', self.opts.max_token_length)? {
			Some((data, b']')) => {
				self.read_single(r)?;
				if data.len() > 0 {
					nend = 0;
				}
				nend += 1;
				return Ok(ST(State::CDataSection(nend), self.flush_text_safe(data)?))
			},
			Some((_, invalid)) => panic!("unexpected delimiter returned: {:?}", invalid),
			None => panic!("loooong cdata text"),
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
	pub fn lex<'r, R: io::BufRead + ?Sized>(&mut self, r: &'r mut R) -> Result<Option<Token>>
	{
		loop {
			let result = match self.state.clone() {
				State::Content(substate) => self.lex_content(substate.clone(), r),
				State::Element{ kind, state: substate } => self.lex_element(kind, substate.clone(), r),
				State::Reference{ ret, kind } => self.lex_reference(ret, kind, r),
				State::CDataSection(nend) => self.lex_cdata_section(nend, r),
			};
			let st = match result {
				Err(e) => match e {
					LexerError::IO(ref sube) if sube.kind() == io::ErrorKind::UnexpectedEof => {
						if self.is_valid_terminal_state() {
							// Important to return here to break out of the loop.
							return Ok(None)
						} else {
							Err(e)
						}
					},
					e => Err(e),
				},
				Err(e) => Err(e),
				Ok(st) => Ok(st),
			}?;
			match st.splice(&mut self.state) {
				Some(tok) => return Ok(Some(tok)),
				None => (),
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

/// Stream tokens to the sink until the end of stream is reached.
fn stream_to_sink<'r, 's, 'l, R: io::BufRead + ?Sized, S: Sink>(l: &'l mut Lexer, r: &'r mut R, s: &'s mut S) -> Result<()> {
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

#[cfg(test)]
mod tests {
	use super::*;
	use std::fmt;
	use std::io::Read;
	use std::error;

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
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(token_limit);
		stream_to_sink(&mut lexer, &mut buff, &mut sink)?;
		Ok(sink.dest)
	}

	#[test]
	fn read_up_to_limited_limits() {
		let mut src = "<?xml version='1.0'?>".as_bytes();
		// use a capacity of 1 to allow limiting to kick in
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut out: Vec<u8> = Vec::new();
		// that is a space
		let result = read_up_to_limited(&mut buffered, &32u8, &mut out, 4);
		assert!(result.is_ok());
		assert_eq!(result.unwrap(), (4, None));
		assert_eq!(out.as_slice(), "<?xm".as_bytes());
	}

	#[test]
	fn read_up_to_limited_finds_delimiter() {
		let mut src = "<?xml version='1.0'?>".as_bytes();
		// use a capacity of 1 to allow limiting to kick in
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut out: Vec<u8> = Vec::new();
		// that is a space
		let result = read_up_to_limited(&mut buffered, &32u8, &mut out, 6);
		assert!(result.is_ok());
		assert_eq!(result.unwrap(), (5, Some(32u8)));
		assert_eq!(out.as_slice(), "<?xml".as_bytes());
	}

	#[test]
	fn read_up_to_limited_uses_buffer_beyond_limit_if_available() {
		let mut src = "<?xml version='1.0'?>".as_bytes();
		// use a capacity of 1 to allow limiting to kick in
		let mut buffered = io::BufReader::with_capacity(8, src);
		let mut out: Vec<u8> = Vec::new();
		// that is a space
		let result = read_up_to_limited(&mut buffered, &32u8, &mut out, 4);
		assert!(result.is_ok());
		assert_eq!(result.unwrap(), (5, Some(32u8)));
		assert_eq!(out.as_slice(), "<?xml".as_bytes());
	}

	#[test]
	fn read_up_to_limited_finds_delimiter_but_does_not_consume_it() {
		let mut src = "<?xml version='1.0'?>".as_bytes();
		// use a capacity of 1 to allow limiting to kick in
		let mut buffered = io::BufReader::with_capacity(8, src);
		let mut out: Vec<u8> = Vec::new();
		// that is a space
		let result = read_up_to_limited(&mut buffered, &32u8, &mut out, 4);
		assert!(result.is_ok());
		assert_eq!(result.unwrap(), (5, Some(32u8)));
		assert_eq!(out.as_slice(), "<?xml".as_bytes());

		let mut out = 0u8;
		assert!(buffered.read_exact(slice::from_mut(&mut out)).is_ok());
		assert_eq!(out, 32u8);
	}

	#[test]
	fn discard_up_to_does_not_discard_match() {
		let mut src = "<?xml version='1.0'?>".as_bytes();
		let mut out: Vec<u8> = Vec::new();
		// that is a space
		let result = discard_up_to(&mut src, &32u8);
		assert!(result.is_ok());
		assert_eq!(result.unwrap(), 32u8);
		let mut out = 0u8;
		assert!(src.read_exact(slice::from_mut(&mut out)).is_ok());
		assert_eq!(out, 32u8);
	}

	#[test]
	fn u8_slice_byteselector_matches_any() {
		let selector = &[b' ', b'x'][..];
		let s1 = b"foobar";
		let s2 = b"yax";
		let s3 = b"foo bar baz";

		assert_eq!(selector.find_any_first_in(s1), None);
		assert_eq!(selector.find_any_first_in(s2), Some((2, b'x')));
		assert_eq!(selector.find_any_first_in(s3), Some((3, b' ')));
	}

	#[test]
	fn u8_byteselector_matches() {
		let selector = b' ';
		let s1 = b"foobar";
		let s2 = b"foo bar baz";

		assert_eq!(selector.find_any_first_in(s1), None);
		assert_eq!(selector.find_any_first_in(s2), Some((3, b' ')));
	}

	#[test]
	fn u8_slice_inverted_byteselector_matches() {
		let selector = InvertDelimiters(&[b'f', b'o']);
		let s1 = b"foobar";
		let s2 = b"foo";
		let s3 = b"foo bar baz";

		assert_eq!(selector.find_any_first_in(s1), Some((3, b'b')));
		assert_eq!(selector.find_any_first_in(s2), None);
		assert_eq!(selector.find_any_first_in(s3), Some((3, b' ')));
	}

	#[test]
	fn lexer_lex_xml_decl_start() {
		let mut src = "<?xml".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink);

		assert_eq!(sink.dest[0], Token::XMLDeclStart);
	}

	#[test]
	fn lexer_lex_xml_decl_version_name() {
		let mut src = "<?xml version=".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink);

		assert_eq!(sink.dest[1], Token::Name("version".to_string()));
	}

	#[test]
	fn lexer_lex_xml_decl_version_eq() {
		let mut src = "<?xml version=".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink);

		assert_eq!(sink.dest[2], Token::Eq);
	}

	#[test]
	fn lexer_lex_xml_decl_version_value_squot() {
		let mut src = "<?xml version='1.0'".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink);

		assert_eq!(sink.dest[3], Token::AttributeValue("1.0".to_string()));
	}

	#[test]
	fn lexer_lex_xml_decl_version_value_dquot() {
		let mut src = "<?xml version=\"1.0\"".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink);

		assert_eq!(sink.dest[3], Token::AttributeValue("1.0".to_string()));
	}

	#[test]
	fn lexer_lex_xml_decl_end() {
		let mut src = "<?xml version=\"1.0\"?>".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink);

		assert_eq!(sink.dest[4], Token::XMLDeclEnd);
	}

	#[test]
	fn lexer_lex_xml_decl_complete() {
		let mut src = "<?xml version=\"1.0\" encoding='utf-8'?>".as_bytes();
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		let result = stream_to_sink(&mut lexer, &mut src, &mut sink);

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
		stream_to_sink(&mut lexer, &mut src, &mut sink);

		assert_eq!(sink.dest[0], Token::ElementHeadStart("element".to_string()));
	}

	#[test]
	fn lexer_lex_element_noattr_empty() {
		let mut src = &b"<element/>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart("element".to_string()));
		assert_eq!(sink.dest[1], Token::ElementHeadClose);
	}

	#[test]
	fn lexer_lex_element_noattr_open() {
		let mut src = &b"<element>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

		assert_eq!(sink.dest[0], Token::ElementHeadStart("element".to_string()));
		assert_eq!(sink.dest[1], Token::ElementHFEnd);
	}

	#[test]
	fn lexer_lex_element_noattr_empty_explicit() {
		let mut src = &b"<element></element>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		let result = stream_to_sink(&mut lexer, &mut src, &mut sink);
		assert!(matches!(result, Err(LexerError::NotWellFormed(_))));
	}

	#[test]
	fn lexer_lex_attribute_amp() {
		let mut src = &b"<root foo='&amp;'>"[..];
		let mut lexer = Lexer::new();
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		stream_to_sink(&mut lexer, &mut src, &mut sink).unwrap();

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
		let result = stream_to_sink(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(LexerError::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_restrict_attribute_name_by_token_length() {
		let src = &b"<a foobar2342='foo'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(LexerError::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_restrict_attribute_value_by_token_length() {
		let src = &b"<a b='foobar2342'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(LexerError::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_restrict_attribute_value_by_token_length_even_with_entities() {
		let src = &b"<a b='foob&amp;r'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		let result = stream_to_sink(&mut lexer, &mut buffered, &mut sink);

		assert!(matches!(result, Err(LexerError::RestrictedXml(_))));
	}

	#[test]
	fn lexer_lex_attribute_value_entities_do_only_count_for_expansion() {
		let src = &b"<a b='foob&amp;'/>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut buffered, &mut sink).unwrap();
	}

	#[test]
	fn lexer_lex_token_length_causes_text_nodes_to_be_split() {
		let src = &b"<a>foo001foo002foo003</a>"[..];
		let mut buffered = io::BufReader::with_capacity(1, src);
		let mut lexer = Lexer::with_options(*LexerOptions::defaults().max_token_length(6));
		let mut sink = VecSink::new(128);
		stream_to_sink(&mut lexer, &mut buffered, &mut sink).unwrap();

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
		assert!(matches!(result, Err(LexerError::NotWellFormed(_))));
	}

	#[test]
	fn fuzz_35cabf8da64df7d1() {
		let src = &b"\x10\x00<!"[..];
		let result = run_fuzz_test(src, 128);
		assert!(result.is_err());
	}

	/* #[test]
	fn fuzz_9bde23591fb17cd7() {
		let src = &b"<\x01\x00m\x00\x00\x02\x00\x00?xml\x20vkrsl\x20<?xml\x20vkrs\x00\x30\x27\x00?>\x0a"[..];
		let result = run_fuzz_test(src, 128);
		assert!(result.is_err());
	} */
}
