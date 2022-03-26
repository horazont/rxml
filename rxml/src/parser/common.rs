use std::io;
#[cfg(not(feature = "mt"))]
use std::rc::Rc;
#[cfg(feature = "mt")]
use std::sync::Arc;

use crate::context;
use crate::error::Result;
use crate::lexer::{Lexer, Token};
use crate::strings::*;

/**
# XML version number

Only version 1.0 is supported.
*/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XMLVersion {
	/// XML Version 1.0
	V1_0,
}

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

/// XML core namespace URI (for the `xml:` prefix)
pub const XMLNS_XML: &'static CDataStr =
	unsafe { std::mem::transmute("http://www.w3.org/XML/1998/namespace") };
/// XML namespace URI (for the `xmlns:` prefix)
pub const XMLNS_XMLNS: &'static CDataStr =
	unsafe { std::mem::transmute("http://www.w3.org/2000/xmlns/") };

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
///
///   [`Error::RestrictedXml`]: crate::Error::RestrictedXml
#[derive(Copy, Debug, Clone, PartialEq, Eq)]
pub struct EventMetrics {
	pub(super) len: usize,
}

impl EventMetrics {
	/// Get the number of bytes used to generate this event.
	pub fn len(&self) -> usize {
		self.len
	}

	// Create new event metrics
	pub const fn new(len: usize) -> EventMetrics {
		EventMetrics { len: len }
	}
}

pub static ZERO_METRICS: EventMetrics = EventMetrics::new(0);

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

/// Wrapper around [`Lexer`](crate::Lexer) and [`std::io::BufRead`] to provide
/// a [`TokenRead`].
pub struct LexerAdapter<R: io::BufRead> {
	lexer: Lexer,
	src: R,
}

impl<R: io::BufRead> LexerAdapter<R> {
	/// Wraps a lexer and a codepoint source
	pub fn new(lexer: Lexer, src: R) -> Self {
		Self {
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
	pub fn get_lexer(&self) -> &Lexer {
		&self.lexer
	}

	/// Return a mutable reference to the lexer
	pub fn get_lexer_mut(&mut self) -> &mut Lexer {
		&mut self.lexer
	}
}

impl<R: io::BufRead> TokenRead for LexerAdapter<R> {
	fn read(&mut self) -> Result<Option<Token>> {
		self.lexer.lex(&mut self.src)
	}
}

/**
Trait for parser-like structs.
*/
pub trait Parse {
	type Output;

	/// Parse a single event using tokens from `r`.
	///
	/// If the end of file has been reached after a document accepted by the
	/// parser, `None` is returned. Otherwise, if the document is still
	/// acceptable the next XML event is returned.
	///
	/// If the document violates a constraint, such as the XML 1.0
	/// grammar or namespacing rules, the corresponding error is returned.
	///
	/// Errors from the token source (such as I/O errors) are forwarded.
	///
	/// **Note:** Exchanging the token source between calls to `parse()` is
	/// possible, but not advisible (if the token source represents a
	/// different document).
	fn parse<R: TokenRead>(&mut self, r: &mut R) -> Result<Option<Self::Output>>;

	/// Release all temporary buffers or other ephemeral allocations
	///
	/// This is sensible to call when it is expected that no more data will be
	/// processed by the parser for a while and the memory is better used
	/// elsewhere.
	fn release_temporaries(&mut self);
}

/**
Trait for things which can be constructed with a [`context::Context`].
*/
pub trait WithContext {
	/// Create a new instance using the given shared context.
	fn with_context(ctx: RcPtr<context::Context>) -> Self;
}
