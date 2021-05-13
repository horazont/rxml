use std::io;
use std::fmt;
use std::error;
use std::string;
use std::result::Result as StdResult;

pub const ERRCTX_UNKNOWN: &'static str = "in unknown context";
pub const ERRCTX_TEXT: &'static str = "in text node";
pub const ERRCTX_ATTVAL: &'static str = "in attribute value";
pub const ERRCTX_NAME: &'static str = "in name";
pub const ERRCTX_ATTNAME: &'static str = "in attribute name";
pub const ERRCTX_NAMESTART: &'static str = "at start of name";
pub const ERRCTX_ELEMENT: &'static str = "in element";
pub const ERRCTX_ELEMENT_FOOT: &'static str = "in element footer";
pub const ERRCTX_ELEMENT_CLOSE: &'static str = "at element close";
pub const ERRCTX_CDATA_SECTION: &'static str = "in CDATA section";
pub const ERRCTX_CDATA_SECTION_START: &'static str = "at CDATA section marker";
pub const ERRCTX_XML_DECL: &'static str = "in XML declaration";
pub const ERRCTX_XML_DECL_START: &'static str = "at start of XML declaration";
pub const ERRCTX_XML_DECL_END: &'static str = "at end of XML declaration";
pub const ERRCTX_REF: &'static str = "in entity or character reference";
pub const ERRCTX_DOCBEGIN: &'static str = "at beginning of document";

#[derive(Debug, Clone)]
pub enum WFError {
	/// Indicate in which state the invalid EOF happened
	InvalidEof(&'static str),
	UndeclaredEntity,
	/// Context, codepoint encountered + whether it came from a character
	/// reference
	InvalidChar(&'static str, u32, bool),
	/// Context, codepoint encountered, expected chars
	UnexpectedChar(&'static str, char, Option<&'static [&'static str]>),
	/// Invalid syntax
	InvalidSyntax(&'static str),
	/// Context, token encountered, expected tokens
	UnexpectedToken(&'static str, &'static str, Option<&'static [&'static str]>),
	DuplicateAttribute,
	ElementMismatch,
}

impl WFError {
	pub fn with_context(self, ctx: &'static str) -> WFError {
		match self {
			WFError::InvalidEof(_) => WFError::InvalidEof(ctx),
			WFError::InvalidChar(_, cp, fromref) => WFError::InvalidChar(ctx, cp, fromref),
			WFError::UnexpectedChar(_, ch, alt) => WFError::UnexpectedChar(ctx, ch, alt),
			WFError::UnexpectedToken(_, tok, alt) => WFError::UnexpectedToken(ctx, tok, alt),
			other => other.clone(),
		}
	}
}

impl fmt::Display for WFError {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		match self {
			WFError::InvalidEof(ctx) => write!(f, "invalid eof {}", ctx),
			WFError::UndeclaredEntity => write!(f, "use of undeclared entity"),
			WFError::InvalidChar(ctx, cp, false) => write!(f, "invalid codepoint U+{:x} {}", cp, ctx),
			WFError::InvalidChar(ctx, cp, true) => write!(f, "character reference expanded to invalid codepoint U+{:x} {}", cp, ctx),
			WFError::UnexpectedChar(ctx, ch, Some(opts)) if opts.len() > 0 => {
				write!(f, "U+{:x} not allowed {} (expected ", *ch as u32, ctx)?;
				if opts.len() == 1 {
					f.write_str(opts[0])?;
					f.write_str(")")
				} else {
					f.write_str("one of: ")?;
					for (i, opt) in opts.iter().enumerate() {
						if i > 0 {
							f.write_str(", ")?;
						}
						f.write_str(*opt)?;
					}
					f.write_str(")")
				}
			},
			WFError::UnexpectedChar(ctx, ch, _) => write!(f, "U+{:x} not allowed {}", *ch as u32, ctx),
			WFError::InvalidSyntax(msg) => write!(f, "invalid syntax: {}", msg),
			WFError::UnexpectedToken(ctx, tok, Some(opts)) if opts.len() > 0 => {
				write!(f, "unexpected {} token {} (expected ", tok, ctx)?;
				if opts.len() == 1 {
					f.write_str(opts[0])?;
					f.write_str(")")
				} else {
					f.write_str("one of: ")?;
					for (i, opt) in opts.iter().enumerate() {
						if i > 0 {
							f.write_str(", ")?;
						}
						f.write_str(*opt)?;
					}
					f.write_str(")")
				}
			},
			WFError::UnexpectedToken(ctx, tok, _) => write!(f, "unexpected {} token {}", tok, ctx),
			WFError::DuplicateAttribute => f.write_str("duplicate attribute"),
			WFError::ElementMismatch => f.write_str("start and end tag do not match"),
		}
	}
}

#[derive(Debug, Clone)]
pub enum NWFError {
	MultiColonName(&'static str),
	EmptyNamePart(&'static str),
	UndeclaredNamesacePrefix(&'static str),
	ReservedNamespacePrefix,
}

impl NWFError {
	pub fn with_context(self, ctx: &'static str) -> NWFError {
		match self {
			Self::MultiColonName(_) => Self::MultiColonName(ctx),
			Self::EmptyNamePart(_) => Self::EmptyNamePart(ctx),
			Self::UndeclaredNamesacePrefix(_) => Self::UndeclaredNamesacePrefix(ctx),
			other => other,
		}
	}
}

impl fmt::Display for NWFError {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		match self {
			Self::MultiColonName(ctx) => write!(f, "more than one colon {} name", ctx),
			Self::EmptyNamePart(ctx) => write!(f, "empty string on one side of the colon {} name", ctx),
			Self::UndeclaredNamesacePrefix(ctx) => write!(f, "use of undeclared namespace prefix {} name", ctx),
			Self::ReservedNamespacePrefix => f.write_str("reserved namespace prefix"),
		}
	}
}

#[derive(Debug)]
pub enum Error {
	IO(io::Error),
	Utf8(string::FromUtf8Error),
	InvalidStartByte(u8),
	InvalidContByte(u8),
	InvalidChar(u32),
	NotWellFormed(WFError),
	NotNamespaceWellFormed(NWFError),
	/// Forbidden element
	RestrictedXml(&'static str),
}

pub type Result<T> = StdResult<T, Error>;

impl Error {
	pub fn io(e: io::Error) -> Error {
		Error::IO(e)
	}

	pub(crate) fn wfeof(ctx: &'static str) -> Error {
		Error::NotWellFormed(WFError::InvalidEof(ctx))
	}

	pub fn with_context(self, ctx: &'static str) -> Self {
		match self {
			Self::NotWellFormed(wf) => Self::NotWellFormed(wf.with_context(ctx)),
			Self::NotNamespaceWellFormed(nwf) => Self::NotNamespaceWellFormed(nwf.with_context(ctx)),
			other => other,
		}
	}
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::io(e)
	}
}

impl From<string::FromUtf8Error> for Error {
	fn from(e: string::FromUtf8Error) -> Error {
		Error::Utf8(e)
	}
}

impl fmt::Display for Error {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		match self {
			Error::NotWellFormed(e) => write!(f, "not-well-formed: {}", e),
			Error::NotNamespaceWellFormed(e) => write!(f, "not namespace-well-formed: {}", e),
			Error::RestrictedXml(msg) => write!(f, "restricted xml: {}", msg),
			Error::InvalidStartByte(b) => write!(f, "invalid utf-8 start byte: \\x{:02x}", b),
			Error::InvalidContByte(b) => write!(f, "invalid utf-8 continuation byte: \\x{:02x}", b),
			Error::InvalidChar(ch) => write!(f, "invalid char: U+{:08x}", ch),
			Error::IO(e) => write!(f, "I/O error: {}", e),
			Error::Utf8(e) => write!(f, "utf8 error: {}", e),
		}
	}
}

impl error::Error for Error {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		match self {
			Error::IO(e) => Some(e),
			Error::Utf8(e) => Some(e),
			Error::NotNamespaceWellFormed(_) |
				Error::NotWellFormed(_) |
				Error::RestrictedXml(_) |
				Error::InvalidStartByte(_) |
				Error::InvalidContByte(_) |
				Error::InvalidChar(_) => None,
		}
	}
}
