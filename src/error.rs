/*!
# Error types

This module holds the error types returned by the various functions of this
crate.
*/
use std::io;
use std::sync::Arc;
use std::fmt;
use std::error;
use std::ops::Deref;
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
pub const ERRCTX_DOCEND: &'static str = "at end of document";

/// Violation of a well-formedness constraint or the XML 1.0 grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum WFError {
	/// End-of-file encountered during a construct where more data was
	/// expected.
	///
	/// The contents are implementation details.
	InvalidEof(&'static str),

	/// Attempt to refer to an undeclared entity.
	///
	/// **Note**: May also be emitted in some cases of malformed entities as
	/// the lexer is very conservative about how many chars are read to
	/// interpret an entity.
	UndeclaredEntity,

	/// Unicode codepoint which is not allowed in XML 1.0 encountered.
	///
	/// The contents are implementation details.
	InvalidChar(&'static str, u32, bool),

	/// Unicode codepoint which was not expected at that point in the
	/// grammar.
	///
	/// The contents are implementation details.
	UnexpectedChar(&'static str, char, Option<&'static [&'static str]>),

	/// Generalized invalid syntactic construct which does not fit into any
	/// of the other categories.
	///
	/// The contents are implementation details.
	InvalidSyntax(&'static str),

	/// Token was not expected by the parser at that point in the grammar.
	///
	/// The contents are implementation details.
	UnexpectedToken(&'static str, &'static str, Option<&'static [&'static str]>),

	/// Attribute was declared multiple times in the same element.
	///
	/// **Note:** This will also be emitted for namespaced attributes which
	/// resolve to the same `(uri, localname)` pair after prefix resolution,
	/// even though that is technically a namespace-well-formedness
	/// constraint.
	DuplicateAttribute,

	/// Ending tag name does not match opening tag.
	ElementMismatch,
}

impl ErrorWithContext for WFError {
	fn with_context(self, ctx: &'static str) -> WFError {
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

/// Violation of a namespace-well-formedness constraint or the Namespaces for
/// XML 1.0 grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum NWFError {
	/// More than one colon encountered in a name.
	///
	/// The contents are implementation details.
	MultiColonName(&'static str),

	/// One side of the colon in a name was empty.
	///
	/// The contents are implementation details.
	EmptyNamePart(&'static str),

	/// Use of an undeclared namespace prefix.
	///
	/// The contents are implementation details.
	UndeclaredNamesacePrefix(&'static str),

	/// Attempt to redefine a reserved namespace prefix.
	ReservedNamespacePrefix,

	/// Local name does not conform to Name production (invalid start char)
	InvalidLocalName(&'static str),

	/// Declared namespace URI is empty
	EmptyNamespaceUri,
}

impl ErrorWithContext for NWFError {
	fn with_context(self, ctx: &'static str) -> NWFError {
		match self {
			Self::MultiColonName(_) => Self::MultiColonName(ctx),
			Self::EmptyNamePart(_) => Self::EmptyNamePart(ctx),
			Self::UndeclaredNamesacePrefix(_) => Self::UndeclaredNamesacePrefix(ctx),
			Self::InvalidLocalName(_) => Self::InvalidLocalName(ctx),
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
			Self::InvalidLocalName(ctx) => write!(f, "local name is invalid {} name", ctx),
			Self::EmptyNamespaceUri => write!(f, "namespace URI is empty"),
		}
	}
}

/// [`std::sync::Arc`]-based around [`std::io::Error`] to allow cloning.
#[derive(Clone)]
pub struct IOErrorWrapper(Arc<io::Error>);

impl IOErrorWrapper {
	fn wrap(e: io::Error) -> IOErrorWrapper {
		IOErrorWrapper(Arc::new(e))
	}
}

impl fmt::Debug for IOErrorWrapper {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		fmt::Debug::fmt(&**self, f)
	}
}

impl fmt::Display for IOErrorWrapper {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		fmt::Display::fmt(&**self, f)
	}
}

impl PartialEq for IOErrorWrapper {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.0, &other.0)
	}
}

impl AsRef<io::Error> for IOErrorWrapper {
	fn as_ref(&self) -> &io::Error {
		&*self.0
	}
}

impl Deref for IOErrorWrapper {
	type Target = io::Error;

	fn deref(&self) -> &io::Error {
		&*self.0
	}
}

impl std::borrow::Borrow<io::Error> for IOErrorWrapper {
	fn borrow(&self) -> &io::Error {
		&*self.0
	}
}

/// Error types which may be returned from the parser or lexer.
///
/// With the exception of [`Error::IO`], all errors are fatal and will be returned indefinitely from the parser or lexer after the first encounter.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
	/// An I/O error was encountered during lexing.
	///
	/// I/O errors are not fatal and may be retried. This is especially important for (but not limited to) [`std::io::ErrorKind::WouldBlock`] errors.
	///
	/// **Note:** When an unexpected end-of-file situation is encountered during parsing or lexing, that is signalled using [`Error::NotWellFormed`] instead of a [`std::io::ErrorKind::UnexpectedEof`] error.
	IO(IOErrorWrapper),

	/// An invalid UTF-8 start byte was encountered during decoding.
	InvalidStartByte(u8),
	/// An invalid UTF-8 continuation byte was encountered during decoding.
	InvalidContByte(u8),
	/// An invalid Unicode scalar value was encountered during decoding.
	InvalidChar(u32),
	/// A violation of the XML 1.0 grammar or a well-formedness constraint was
	/// encountered during parsing or lexing.
	NotWellFormed(WFError),
	/// A violation of the Namespaces in XML 1.0 grammar or a
	/// namespace-well-formedness constraint was encountered during parsing.
	NotNamespaceWellFormed(NWFError),
	/// A forbidden construct was encountered during lexing or parsing.
	///
	/// The string indicates the context and should not be interpreted by user
	/// code.
	RestrictedXml(&'static str),
}

pub type Result<T> = StdResult<T, Error>;

pub(crate) trait ErrorWithContext {
	fn with_context(self, ctx: &'static str) -> Self;
}

impl Error {
	pub fn io(e: io::Error) -> Error {
		Error::IO(IOErrorWrapper::wrap(e))
	}

	pub(crate) fn wfeof(ctx: &'static str) -> Error {
		Error::NotWellFormed(WFError::InvalidEof(ctx))
	}
}

impl ErrorWithContext for Error {
	fn with_context(self, ctx: &'static str) -> Self {
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

impl From<WFError> for Error {
	fn from(e: WFError) -> Error {
		Error::NotWellFormed(e)
	}
}

impl From<NWFError> for Error {
	fn from(e: NWFError) -> Error {
		Error::NotNamespaceWellFormed(e)
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
		}
	}
}

impl error::Error for Error {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		match self {
			Error::IO(e) => Some(&**e),
			Error::NotNamespaceWellFormed(_) |
				Error::NotWellFormed(_) |
				Error::RestrictedXml(_) |
				Error::InvalidStartByte(_) |
				Error::InvalidContByte(_) |
				Error::InvalidChar(_) => None,
		}
	}
}
