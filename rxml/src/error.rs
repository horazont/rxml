/*!
# Error types

This module holds the error types returned by the various functions of this
crate.
*/
use std::error;
use std::fmt;
use std::io;
use std::ops::Deref;
use std::result::Result as StdResult;
use std::sync::Arc;

use rxml_validation::Error as ValidationError;

pub(crate) use crate::errctx::*;

/// Violation of a well-formedness or namespace-well-formedness constraint or
/// the XML 1.0 grammar.
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum XmlError {
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

	/// Byte which was not expected at that point in the grammar.
	///
	/// The contents are implementation details.
	UnexpectedByte(&'static str, u8, Option<&'static [&'static str]>),

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
	/// resolve to the same `(uri, localname)` pair after prefix resolution.
	DuplicateAttribute,

	/// Ending tag name does not match opening tag.
	ElementMismatch,

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
	UndeclaredNamespacePrefix(&'static str),

	/// Attempt to redefine a reserved namespace prefix.
	ReservedNamespacePrefix,

	/// Attempt to bind a reserved namespace name to the wrong prefix.
	ReservedNamespaceName,

	/// Local name does not conform to Name production (invalid start char)
	InvalidLocalName(&'static str),

	/// Declared namespace URI is empty
	EmptyNamespaceUri,
}

impl error::Error for XmlError {}

impl ErrorWithContext for XmlError {
	fn with_context(self, ctx: &'static str) -> Self {
		match self {
			Self::InvalidEof(_) => Self::InvalidEof(ctx),
			Self::InvalidChar(_, cp, fromref) => Self::InvalidChar(ctx, cp, fromref),
			Self::UnexpectedChar(_, ch, alt) => Self::UnexpectedChar(ctx, ch, alt),
			Self::UnexpectedToken(_, tok, alt) => Self::UnexpectedToken(ctx, tok, alt),
			Self::MultiColonName(_) => Self::MultiColonName(ctx),
			Self::EmptyNamePart(_) => Self::EmptyNamePart(ctx),
			Self::UndeclaredNamespacePrefix(_) => Self::UndeclaredNamespacePrefix(ctx),
			Self::InvalidLocalName(_) => Self::InvalidLocalName(ctx),
			other => other,
		}
	}
}

impl fmt::Display for XmlError {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		match self {
			Self::InvalidEof(ctx) => write!(f, "invalid eof {}", ctx),
			Self::UndeclaredEntity => write!(f, "use of undeclared entity"),
			Self::InvalidChar(ctx, cp, false) => {
				write!(f, "invalid codepoint U+{:x} {}", cp, ctx)
			}
			Self::InvalidChar(ctx, cp, true) => write!(
				f,
				"character reference expanded to invalid codepoint U+{:x} {}",
				cp, ctx
			),
			Self::UnexpectedChar(ctx, ch, Some(opts)) if opts.len() > 0 => {
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
			}
			Self::UnexpectedByte(ctx, b, Some(opts)) if opts.len() > 0 => {
				write!(f, "0x{:x} not allowed {} (expected ", *b, ctx)?;
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
			}
			Self::UnexpectedChar(ctx, ch, _) => {
				write!(f, "U+{:x} not allowed {}", *ch as u32, ctx)
			}
			Self::UnexpectedByte(ctx, b, _) => write!(f, "0x{:x} not allowed {}", *b, ctx),
			Self::InvalidSyntax(msg) => write!(f, "invalid syntax: {}", msg),
			Self::UnexpectedToken(ctx, tok, Some(opts)) if opts.len() > 0 => {
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
			}
			Self::UnexpectedToken(ctx, tok, _) => write!(f, "unexpected {} token {}", tok, ctx),
			Self::DuplicateAttribute => f.write_str("duplicate attribute"),
			Self::ElementMismatch => f.write_str("start and end tag do not match"),
			Self::MultiColonName(ctx) => write!(f, "more than one colon {} name", ctx),
			Self::EmptyNamePart(ctx) => {
				write!(f, "empty string on one side of the colon {} name", ctx)
			}
			Self::UndeclaredNamespacePrefix(ctx) => {
				write!(f, "use of undeclared namespace prefix {} name", ctx)
			}
			Self::ReservedNamespacePrefix => f.write_str("reserved namespace prefix"),
			Self::ReservedNamespaceName => f.write_str("reserved namespace URI"),
			Self::InvalidLocalName(ctx) => write!(f, "local name is invalid {} name", ctx),
			Self::EmptyNamespaceUri => write!(f, "namespace URI is empty"),
		}
	}
}

impl From<ValidationError> for XmlError {
	fn from(other: ValidationError) -> Self {
		match other {
			ValidationError::EmptyName => Self::InvalidSyntax("Name must have at least one Char"),
			ValidationError::InvalidChar(ch) => Self::UnexpectedChar(ERRCTX_UNKNOWN, ch, None),
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
	/// **Note:** When an unexpected end-of-file situation is encountered during parsing or lexing, that is signalled using [`Error::Xml`] instead of a [`std::io::ErrorKind::UnexpectedEof`] error.
	IO(IOErrorWrapper),

	/// An invalid UTF-8 byte was encountered during decoding.
	InvalidUtf8Byte(u8),
	/// An invalid Unicode scalar value was encountered during decoding.
	InvalidChar(u32),
	/// A violation of the XML 1.0 grammar or a well-formedness or
	/// namespace-well-formedness constraint was encountered during parsing or
	/// lexing.
	Xml(XmlError),
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
		Self::Xml(XmlError::InvalidEof(ctx))
	}
}

impl ErrorWithContext for Error {
	fn with_context(self, ctx: &'static str) -> Self {
		match self {
			Self::Xml(xe) => Self::Xml(xe.with_context(ctx)),
			other => other,
		}
	}
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::io(e)
	}
}

impl From<XmlError> for Error {
	fn from(e: XmlError) -> Self {
		Self::Xml(e)
	}
}

impl fmt::Display for Error {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		match self {
			Self::Xml(e) => write!(f, "xml error: {}", e),
			Self::RestrictedXml(msg) => write!(f, "restricted xml: {}", msg),
			Self::InvalidUtf8Byte(b) => write!(f, "invalid utf-8 byte: \\x{:02x}", b),
			Self::InvalidChar(ch) => write!(f, "invalid char: U+{:08x}", ch),
			Self::IO(e) => write!(f, "I/O error: {}", e),
		}
	}
}

impl error::Error for Error {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		match self {
			Self::IO(e) => Some(&**e),
			Self::Xml(e) => Some(e),
			Self::RestrictedXml(_) | Self::InvalidUtf8Byte(_) | Self::InvalidChar(_) => None,
		}
	}
}

pub(crate) fn add_context<T, E: ErrorWithContext>(
	r: StdResult<T, E>,
	ctx: &'static str,
) -> StdResult<T, E> {
	r.or_else(|e| Err(e.with_context(ctx)))
}
