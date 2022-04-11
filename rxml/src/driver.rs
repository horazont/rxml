/*!
Wrappers around lexers and parsers to drive them.

For high-level parsing, [`FeedParser`] and [`PullParser`] are the things to
look at. More information and examples can also be found in the [`rxml`]
top-level documentation.

   [`rxml`]: crate
*/

use std::io;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::lexer::{Lexer, LexerOptions};
use crate::parser;
use crate::parser::{BufferLexerAdapter, LexerAdapter, Parse, Parser};

/**
# Source for individual XML events

This trait is implemented by the different parser frontends. It is analogous
to the [`std::io::Read`] trait, but for XML document events instead of bytes.
*/
pub trait EventRead {
	type Output;

	/// Read a single event from the parser.
	///
	/// If the EOF has been reached with a valid document, `None` is returned.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	fn read(&mut self) -> Result<Option<Self::Output>>;

	/// Read all events which can be produced from the data source (at this
	/// point in time).
	///
	/// The given `cb` is invoked for each event.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	fn read_all<F>(&mut self, mut cb: F) -> Result<()>
	where
		F: FnMut(Self::Output) -> (),
	{
		loop {
			match self.read()? {
				None => return Ok(()),
				Some(ev) => cb(ev),
			}
		}
	}

	/// Read all events which can be produced from the data source (at this
	/// point in time).
	///
	/// The given `cb` is invoked for each event.
	///
	/// If the data source indicates that it needs to block to read further
	/// data, `false` is returned. If the EOF is reached successfully, `true`
	/// is returned.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	fn read_all_eof<F>(&mut self, cb: F) -> Result<bool>
	where
		F: FnMut(Self::Output) -> (),
	{
		as_eof_flag(self.read_all(cb))
	}
}

/**
# Non-blocking driver for parsers

This is a generic non-blocking push-based driver for objects implementing the
[`Parse`] trait.

In general, it is advised to use the [`FeedParser`] alias which specializes
this struct for use with the default [`Parser`].
*/
pub struct PushDriver<P: Parse> {
	parser: P,
	lexer: Lexer,
}

/// Convert end-of-file-ness of a result to a boolean flag.
///
/// If the result is ok, return true (EOF). If the result is not ok, but the
/// error is an I/O error indicating that the data source would have to block
/// to read further data, return false ("Ok, but not at eof yet").
///
/// All other errors are passed through.
pub fn as_eof_flag(r: Result<()>) -> Result<bool> {
	match r {
		Err(Error::IO(ioerr)) if ioerr.kind() == io::ErrorKind::WouldBlock => Ok(false),
		Err(e) => Err(e),
		Ok(()) => Ok(true),
	}
}

impl<P: Parse + Default> Default for PushDriver<P> {
	/// Create a new push driver using the defaults for its parser and lexer.
	fn default() -> Self {
		Self::wrap(Lexer::new(), P::default())
	}
}

impl<P: Parse + Default> PushDriver<P> {
	#[deprecated(since = "0.7.0", note = "use the Default trait implementation instead")]
	pub fn new() -> Self {
		Self::default()
	}
}

impl<P: Parse + parser::WithContext> parser::WithContext for PushDriver<P> {
	/// Create a new PushDriver, using the given context for the parser.
	fn with_context(ctx: parser::RcPtr<Context>) -> Self {
		Self::wrap(Lexer::new(), P::with_context(ctx))
	}
}

impl<P: Parse> PushDriver<P> {
	/// Compose a new PushDriver from parts
	pub fn wrap(lexer: Lexer, parser: P) -> Self {
		Self { parser, lexer }
	}

	/// Parse bytes from a buffer, until either an error occurs, a valid
	/// event is emitted or EOF is reached.
	///
	/// This function is intended for parsing a document without keeping it
	/// fully in memory and without blocking on the input. As it is fed
	/// buffers, it is not possible to distinguish the case where the end of
	/// a buffer is reached vs. the case where the end of the input overall
	/// has been reached from just looking at the buffer itself.
	///
	/// The `at_eof` argument is thus required to signal to the parser whether
	/// the end of the passed buffer is identical to the end of the complete
	/// document.
	///
	/// If the end of the buffer is reached while `at_eof` is false, an I/O
	/// error of kind [`std::io::ErrorKind::WouldBlock`] is emitted.
	pub fn parse<T: bytes::Buf>(
		&mut self,
		data: &mut T,
		at_eof: bool,
	) -> Result<Option<P::Output>> {
		self.parser.parse(&mut BufferLexerAdapter {
			lexer: &mut self.lexer,
			buf: data,
			eof: at_eof,
		})
	}

	/// Parse all data from the given buffer and pass the generated events to
	/// a callback.
	///
	/// In contrast to [`parse()`], on success, this always consumes the
	/// entire buffer. Events which are encountered while processing the
	/// buffer are handed to the given callback.
	///
	/// The end-of-file behaviour is identical to [`parse()`].
	///
	/// To obtain a boolean flag instead of an I/O error on end of file, pass
	/// the result through [`as_eof_flag`].
	///
	///    [`parse()`]: Self::parse
	pub fn parse_all<T: bytes::Buf, F: FnMut(P::Output) -> ()>(
		&mut self,
		data: &mut T,
		at_eof: bool,
		mut f: F,
	) -> Result<()> {
		loop {
			match self.parse(data, at_eof)? {
				None => return Ok(()),
				Some(ev) => f(ev),
			}
		}
	}

	/// Access the lexer
	pub fn get_lexer(&self) -> &Lexer {
		&self.lexer
	}

	/// Access the lexer, mutably
	pub fn get_lexer_mut(&mut self) -> &mut Lexer {
		&mut self.lexer
	}

	/// Access the parser
	pub fn get_parser(&self) -> &P {
		&self.parser
	}

	/// Access the parser, mutably
	pub fn get_parser_mut(&mut self) -> &mut P {
		&mut self.parser
	}

	/// Decompose the driver into the inner lexer and the inner parser.
	pub fn into_inner(self) -> (Lexer, P) {
		(self.lexer, self.parser)
	}

	/// Release all temporary buffers
	///
	/// This is sensible to call when it is expected that no more data will be
	/// processed by the parser for a while and the memory is better used
	/// elsewhere.
	pub fn release_temporaries(&mut self) {
		self.get_lexer_mut().release_temporaries();
		self.get_parser_mut().release_temporaries();
	}
}

/**
# Blocking driver for parsers

This is a generic blocking pull-based driver for objects implementing the
[`Parse`] trait.

In general, it is advised to use the [`PullParser`] alias which specializes
this struct for use with the default [`Parser`].
*/
pub struct PullDriver<T: io::BufRead, P: Parse> {
	parser: P,
	token_source: LexerAdapter<T>,
}

impl<T: io::BufRead, P: Parse + Default> PullDriver<T, P> {
	/// Create a new parser with default options, wrapping the given reader.
	pub fn new(inner: T) -> Self {
		Self::with_options(inner, LexerOptions::default())
	}

	/// Create a new parser while configuring the lexer with the given
	/// options.
	pub fn with_options(inner: T, options: LexerOptions) -> Self {
		Self::wrap(inner, Lexer::with_options(options), P::default())
	}
}

impl<T: io::BufRead, P: Parse> PullDriver<T, P> {
	/// Create a fully customized parser from a lexer and a parser component.
	pub fn wrap(inner: T, lexer: Lexer, parser: P) -> Self {
		Self {
			token_source: LexerAdapter::new(lexer, inner),
			parser,
		}
	}
	/// Access the inner BufRead
	pub fn get_inner(&self) -> &T {
		self.token_source.get_ref()
	}

	/// Access the inner BufRead, mutably
	pub fn get_inner_mut(&mut self) -> &mut T {
		self.token_source.get_mut()
	}

	/// Access the lexer
	pub fn get_lexer(&self) -> &Lexer {
		self.token_source.get_lexer()
	}

	/// Access the lexer, mutably
	pub fn get_lexer_mut(&mut self) -> &mut Lexer {
		self.token_source.get_lexer_mut()
	}

	/// Access the parser
	pub fn get_parser(&self) -> &P {
		&self.parser
	}

	/// Access the parser, mutably
	pub fn get_parser_mut(&mut self) -> &mut P {
		&mut self.parser
	}
}

impl<T: io::BufRead, P: Parse> EventRead for PullDriver<T, P> {
	type Output = P::Output;

	/// Read a single event from the parser.
	///
	/// If the EOF has been reached with a valid document, `None` is returned.
	///
	/// All I/O errors from the source are passed on without modification.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	fn read(&mut self) -> Result<Option<Self::Output>> {
		self.parser.parse(&mut self.token_source)
	}
}

/**
# Non-blocking parsing

The [`FeedParser`] allows parsing XML documents as they arrive in the
application, giving back control to the caller immediately when not enough
data is available for processing. This is especially useful when streaming
data from sockets.

This is a type alias around a [`PushDriver`] and documentation for the API is
found there.

To read events from the `FeedParser` after feeding data, use its [`EventRead`]
trait.

## Example

```
use rxml::{FeedParser, Error, ResolvedEvent, XMLVersion, EventRead};
use std::io;
let doc = b"<?xml version='1.0'?><hello>World!</hello>";
let mut fp = FeedParser::new();
// We expect a WouldBlock, because the XML declaration is not complete yet
assert!(matches!(
	fp.parse(&mut &doc[..10], false).err().unwrap(),
	Error::IO(e) if e.kind() == io::ErrorKind::WouldBlock
));

// Now we pass the XML declaration (and some), so we expect a corresponding
// event
let ev = fp.parse(&mut &doc[10..25], false);
assert!(matches!(ev.unwrap().unwrap(), ResolvedEvent::XMLDeclaration(_, XMLVersion::V1_0)));
```

## Parsing without namespace expansion

To parse an XML document without namespace expansion in non-blocking mode,
one can use the [`PushDriver`] with a [`RawParser`]. Note the caveats in the
[`RawParser`] documentation before using it!

   [`RawParser`]: crate::parser::RawParser
*/
pub type FeedParser = PushDriver<Parser>;

/**
# Blocking parsing

The [`PullParser`] allows parsing XML documents from a [`io::Read`]
blockingly. The parser will block until the backing [`io::Read`] has enough
data available (or returns an error).

This is a type alias around a [`PullDriver`] and documentation for the API is
found there.

Interaction with a `PullParser` should happen exclusively via the
[`EventRead`] trait.

## Blocking I/O

If the [`PullParser`] is used with blocking I/O and a source which may block for a significant amount of time (e.g. a network socket), some events may be emitted with significant delay. This is due to an edge case where the lexer may emit a token without consuming a byte from the source.

This internal state of the lexer is not observable from the outside, but it affects most importantly closing element tags. In practice, this means that the last closing element tag of a "stanza" of XML is only going to be emitted once the first byte of the next stanza has been made available through the BufRead.

This only affects blocking I/O, because a non-blocking source will return [`std::io::ErrorKind::WouldBlock`] from the read call and yield control back to the parser to emit the event.

In general, for networked operations, it is recommended to use the [`FeedParser`] or [`AsyncParser`] instead of the [`PullParser`].

## Example

```
use rxml::{PullParser, Error, ResolvedEvent, XMLVersion, EventRead};
use std::io;
use std::io::BufRead;
let mut doc = &b"<?xml version='1.0'?><hello>World!</hello>"[..];
// this converts the doc into an io::BufRead
let mut pp = PullParser::new(&mut doc);
// we expect the first event to be the XML declaration
let ev = pp.read();
assert!(matches!(ev.unwrap().unwrap(), ResolvedEvent::XMLDeclaration(_, XMLVersion::V1_0)));
```

## Parsing without namespace expansion

To parse an XML document without namespace expansion in blocking mode,
one can use the [`PullDriver`] with a [`RawParser`]. Note the caveats in the
[`RawParser`] documentation before using it!

   [`RawParser`]: crate::parser::RawParser
   [`AsyncParser`]: crate::AsyncParser
*/
pub type PullParser<T> = PullDriver<T, Parser>;
