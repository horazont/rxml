/*!
# Restricted XML parsing

This crate provides "restricted" parsing of XML 1.0 documents with
namespacing.

## Features (some call them restrictions)

* No external resources
* No custom entities
* No DTD whatsoever
* No processing instructions
* No comments
* UTF-8 input only
* Namespacing-well-formedness enforced
* XML 1.0 only
* Streamed parsing (parser emits a subset of SAX events)
* Can be driven push- and pull-based
* Tokio-based asynchronicity supported via the `async` feature and [`AsyncParser`].

## Example

```
use rxml::EventRead;
let doc = b"<?xml version='1.0'?><hello>World!</hello>";
let mut fp = rxml::FeedParser::new();
fp.feed(doc.to_vec());
fp.feed_eof();
let result = fp.read_all_eof(|ev| {
	println!("got event: {:?}", ev);
});
// true indicates eof
assert_eq!(result.unwrap(), true);
```

## High-level usage

### Push-based usage

The [`FeedParser`] allows to push bits of XML into the parser as they arrive
in the application and process the resulting [`Event`]s as they happen.

### Pull-based usage

If the parser should block while waiting for more data to arrive, a
[`PullParser`] can be used instead. The `PullParser` requires a source which
implements [`io::BufRead`].

### Usage with Tokio

Tokio is supported with the `async` feature. It offers the [`AsyncParser`]
and the [`AsyncEventRead`] trait, which work similar to the `PullParser`.
Instead of blocking, however, the async parser will yield control to other
tasks.
*/
#[allow(unused_imports)]
use std::io;

pub mod error;
pub mod lexer;
pub mod parser;
mod bufq;
pub mod strings;
mod context;
mod errctx;
pub mod writer;

#[cfg(test)]
pub mod tests;

#[doc(inline)]
pub use error::{Error, Result};
#[doc(inline)]
pub use lexer::{Lexer, LexerOptions};
#[doc(inline)]
pub use parser::{QName, Parser, Event, LexerAdapter, XMLVersion, XMLNS_XML};
#[doc(inline)]
pub use writer::{Encoder, Item};
#[doc(inline)]
pub use bufq::BufferQueue;
pub use strings::{NCName, Name, NCNameStr, NameStr, CData, CDataStr};
pub use context::Context;

#[cfg(feature = "async")]
use {
	tokio::io::{AsyncBufRead, AsyncBufReadExt},
	async_trait::async_trait,
};

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

/**
# Source for individual XML events

This trait is implemented by the different parser frontends. It is analogous
to the [`std::io::Read`] trait, but for [`Event`]s instead of bytes.
*/
pub trait EventRead {
	/// Read a single event from the parser.
	///
	/// If the EOF has been reached with a valid document, `None` is returned.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	fn read(&mut self) -> Result<Option<Event>>;

	/// Read all events which can be produced from the data source (at this
	/// point in time).
	///
	/// The given `cb` is invoked for each event.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	fn read_all<F>(&mut self, mut cb: F) -> Result<()>
		where F: FnMut(Event) -> ()
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
		where F: FnMut(Event) -> ()
	{
		as_eof_flag(self.read_all(cb))
	}
}

/**
# Non-blocking parsing

The [`FeedParser`] allows parsing XML documents as they arrive in the
application, giving back control to the caller immediately when not enough
data is available for processing. This is especially useful when streaming
data from sockets.

To read events from the `FeedParser` after feeding data, use its [`EventRead`]
trait.

## Example

```
use rxml::{FeedParser, Error, Event, XMLVersion, EventRead};
use std::io;
let doc = b"<?xml version='1.0'?><hello>World!</hello>";
let mut fp = FeedParser::new();
fp.feed(doc[..10].to_vec());
// We expect a WouldBlock, because the XML declaration is not complete yet
let ev = fp.read();
assert!(matches!(
    ev.err().unwrap(),
    Error::IO(e) if e.kind() == io::ErrorKind::WouldBlock
));

fp.feed(doc[10..25].to_vec());
// Now we passed the XML declaration (and some), so we expect a corresponding
// event
let ev = fp.read();
assert!(matches!(ev.unwrap().unwrap(), Event::XMLDeclaration(_, XMLVersion::V1_0)));
```
*/
pub struct FeedParser<'x> {
	token_source: LexerAdapter<BufferQueue<'x>>,
	parser: Parser,
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

impl<'x> FeedParser<'x> {
	/// Create a new default `FeedParser`.
	pub fn new() -> FeedParser<'x> {
		Self::with_context(parser::RcPtr::new(Context::new()))
	}

	pub fn with_context(ctx: parser::RcPtr<Context>) -> FeedParser<'x> {
		FeedParser{
			token_source: LexerAdapter::new(
				Lexer::new(),
				BufferQueue::new(),
			),
			parser: Parser::with_context(ctx),
		}
	}

	/// Feed a chunck of data to the parser.
	///
	/// This enqueues the data for processing, but does not process it right
	/// away.
	///
	/// To process data, call [`FeedParser::read()`] or
	/// [`FeedParser::read_all()`].
	///
	/// # Panics
	///
	/// If [`FeedParser::feed_eof()`] has been called before.
	pub fn feed<'a: 'x, T: Into<std::borrow::Cow<'a, [u8]>>>(&mut self, data: T)
	{
		self.token_source.get_mut().push(data);
	}

	/// Feed the eof marker to the parser.
	///
	/// This is a prerequisite for parsing to terminate with an eof signal
	/// (returning `true`). Otherwise, `false` will be returned indefinitely
	/// without emitting any events.
	///
	/// After the eof marker has been fed to the parser, no further data can
	/// be fed.
	pub fn feed_eof(&mut self) {
		self.token_source.get_mut().push_eof();
	}

	/// Return the amount of bytes which have not been read from the buffer
	/// yet.
	///
	/// This may not reflect the amount of memory used by the buffer
	/// accurately, as memory is only released when an entire chunk (as fed
	/// to `feed()`) has been processed (and only if that chunk is owned by
	/// the parser).
	pub fn buffered(&self) -> usize {
		self.token_source.get_ref().len()
	}

	/// Return a reference to the internal buffer BufferQueue
	///
	/// This can be used to force dropping of all memory in case of error
	/// conditions.
	pub fn get_buffer_mut(&mut self) -> &mut BufferQueue<'x> {
		self.token_source.get_mut()
	}

	/// Release all temporary buffers
	///
	/// This is sensible to call when it is expected that no more data will be
	/// processed by the parser for a while and the memory is better used
	/// elsewhere.
	pub fn release_temporaries(&mut self) {
		self.token_source.get_lexer_mut().release_temporaries();
		self.parser.release_temporaries();
	}
}

impl EventRead for FeedParser<'_> {
	/// Read a single event from the parser.
	///
	/// If the EOF has been reached with a valid document, `None` is returned.
	///
	/// If the buffered data is not sufficient to create an event, an I/O
	/// error of [`std::io::ErrorKind::WouldBlock`] is returned.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	fn read(&mut self) -> Result<Option<Event>> {
		self.parser.parse(&mut self.token_source)
	}
}

/**
# Blocking parsing

The [`PullParser`] allows parsing XML documents from a [`io::Read`]
blockingly. The parser will block until the backing [`io::Read`] has enough
data available (or returns an error).

Interaction with a `PullParser` should happen exclusively via the
[`EventRead`] trait.

## Blocking I/O

If the [`PullParser`] is used with blocking I/O and a source which may block for a significant amount of time (e.g. a network socket), some events may be emitted with significant delay. This is due to an edge case where the lexer may emit a token without consuming a byte from the source.

This internal state of the lexer is not observable from the outside, but it affects most importantly closing element tags. In practice, this means that the last closing element tag of a "stanza" of XML is only going to be emitted once the first byte of the next stanza has been made available through the BufRead.

This only affects blocking I/O, because a non-blocking source will return [`std::io::ErrorKind::WouldBlock`] from the read call and yield control back to the parser to emit the event.

In general, for networked operations, it is recommended to use the [`FeedParser`] or [`AsyncParser`] instead of the [`PullParser`].

## Example

```
use rxml::{PullParser, Error, Event, XMLVersion, EventRead};
use std::io;
use std::io::BufRead;
let mut doc = &b"<?xml version='1.0'?><hello>World!</hello>"[..];
// this converts the doc into an io::BufRead
let mut pp = PullParser::new(&mut doc);
// we expect the first event to be the XML declaration
let ev = pp.read();
assert!(matches!(ev.unwrap().unwrap(), Event::XMLDeclaration(_, XMLVersion::V1_0)));
```
*/
pub struct PullParser<T: io::BufRead> {
	parser: Parser,
	token_source: LexerAdapter<T>,
}

impl<T: io::BufRead> PullParser<T> {
	/// Create a new PullParser, wrapping the given reader.
	///
	/// **Note:** It is highly recommended to wrap a common reader into
	/// [`std::io::BufReader`] as the implementation will do lots of small
	/// `read()` calls. Those would be terribly inefficient without buffering.
	pub fn new(r: T) -> Self {
		PullParser{
			token_source: LexerAdapter::new(
				Lexer::new(),
				r,
			),
			parser: Parser::new(),
		}
	}
}

impl<T: io::BufRead> EventRead for PullParser<T> {
	/// Read a single event from the parser.
	///
	/// If the EOF has been reached with a valid document, `None` is returned.
	///
	/// All I/O errors from the source are passed on without modification.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	fn read(&mut self) -> Result<Option<Event>> {
		self.parser.parse(&mut self.token_source)
	}
}

/**
# Asynchronous source for individual XML events

This trait is implemented by the different parser frontends. It is analogous
to the [`tokio::io::AsyncRead`] trait, but for [`Event`]s instead of bytes.
*/
#[cfg(feature = "async")]
#[async_trait]
pub trait AsyncEventRead {
	/// Read a single event from the parser.
	///
	/// If the EOF has been reached with a valid document, `None` is returned.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	///
	/// Equivalent to:
	///
	/// ```ignore
	/// async fn read(&mut self) -> Result<Option<Event>>;
	/// ```
	async fn read(&mut self) -> Result<Option<Event>>;

	/// Read all events which can be produced from the data source (at this
	/// point in time).
	///
	/// The given `cb` is invoked for each event.
	///
	/// I/O errors may be retried, all other errors are fatal (and will be
	/// returned again by the parser on the next invocation without reading
	/// further data from the source).
	///
	/// Equivalent to:
	///
	/// ```ignore
	/// 	async fn read_all<F>(&mut self, mut cb: F) -> Result<()>
	///			where F: FnMut(Event) -> () + Send
	/// ```
	async fn read_all<F>(&mut self, mut cb: F) -> Result<()>
		where F: FnMut(Event) -> () + Send
	{
		loop {
			match self.read().await? {
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
	///
	/// Equivalent to:
	///
	/// ```ignore
	/// 	async fn read_all_eof<F>(&mut self, cb: F) -> Result<bool>
	///			where F: FnMut(Event) -> () + Send
	/// ```
	async fn read_all_eof<F>(&mut self, cb: F) -> Result<bool>
		where F: FnMut(Event) -> () + Send
	{
		as_eof_flag(self.read_all(cb).await)
	}
}

/**
# Asynchronous parsing

The [`AsyncParser`] allows parsing XML documents from a [`tokio::io::AsyncBufRead`], asynchronously. It operates similarly as the [`PullParser`] does, but instead of blocking the task, it will yield control to other tasks if the backend is not able to supply data immediately.

Interaction with a `AsyncParser` should happen exclusively via the [`AsyncEventRead`] trait.

## Example

The example is a bit pointless because it does not really demonstrate the asynchronicity.

```
use rxml::{AsyncParser, Error, Event, XMLVersion, AsyncEventRead};
use tokio::io::AsyncRead;
# tokio_test::block_on(async {
let mut doc = &b"<?xml version='1.0'?><hello>World!</hello>"[..];
// this converts the doc into an tokio::io::AsyncRead
let mut pp = AsyncParser::new(&mut doc);
// we expect the first event to be the XML declaration
let ev = pp.read().await;
assert!(matches!(ev.unwrap().unwrap(), Event::XMLDeclaration(_, XMLVersion::V1_0)));
# })
*/
#[cfg(feature = "async")]
pub struct AsyncParser<T: AsyncBufRead + Unpin> {
	reader: T,
	lexer: Lexer,
	parser: Parser,
	blocked: bool,
}

#[cfg(feature = "async")]
struct AsyncLexerAdapter<'x, 'y> {
	lexer: &'x mut Lexer,
	buf: &'x mut &'y [u8],
	blocked: &'x mut bool,
}

#[cfg(feature = "async")]
impl<'x, 'y> parser::TokenRead for AsyncLexerAdapter<'x, 'y> {
	fn read(&mut self) -> Result<Option<lexer::Token>> {
		match self.lexer.lex_bytes(self.buf, false) {
			Err(Error::IO(ioerr)) => {
				*self.blocked = true;
				Err(Error::IO(ioerr))
			},
			other => {
				*self.blocked = false;
				other
			},
		}
	}
}

#[cfg(feature = "async")]
struct EofLexerAdapter<'x> {
	lexer: &'x mut Lexer,
}

#[cfg(feature = "async")]
impl<'x> parser::TokenRead for EofLexerAdapter<'x> {
	fn read(&mut self) -> Result<Option<lexer::Token>> {
		let mut buf: &'static [u8] = &[];
		self.lexer.lex_bytes(&mut buf, true)
	}
}

#[cfg(feature = "async")]
impl<T: AsyncBufRead + Unpin + Send> AsyncParser<T> {
	pub fn new(r: T) -> Self {
		Self{
			reader: r,
			lexer: Lexer::new(),
			parser: Parser::new(),
			blocked: true,
		}
	}

	#[inline]
	fn parse(lexer: &mut Lexer, parser: &mut Parser, buf: &mut &[u8], blocked: &mut bool) -> (usize, Option<Result<Option<Event>>>) {
		let old_len = buf.len();
		let result = parser.parse(&mut AsyncLexerAdapter{
			lexer,
			buf,
			blocked,
		});
		let new_len = buf.len();
		let read = old_len - new_len;
		match result {
			Ok(v) => return (read, Some(Ok(v))),
			Err(Error::IO(ioerr)) if ioerr.kind() == io::ErrorKind::WouldBlock => (read, None),
			Err(e) => return (read, Some(Err(e))),
		}
	}
}

#[cfg(feature = "async")]
#[async_trait]
impl<T: AsyncBufRead + Unpin + Send> AsyncEventRead for AsyncParser<T> {
	async fn read(&mut self) -> Result<Option<Event>> {
		// The blocked flag is controlled by the AsyncLexerAdapter. If the lexer returns with an I/O error, we set this flag to true (i.e. re-try I/O immediately).
		// If the lexer returns anything else (i.e. a non-I/O error or a token or eof), we set the flag to false, causing an empty-buffer-read to be performed before trying I/O on the source.
		// `blocked` is also automatically false on parser errors because:
		// - the parser *always* needs a token (or eof) to cause an error in the first place
		// - to obtain a token or eof, the lexer needs to return one
		// - if the lexer returns token or eof, blocked is set to false
		// - on a parser error, the parser does not call the lexer, so it cannot flip the blocked bit anymore
		// As a third case, `blocked` is also automatically false if the parser has more events to emit from the same token:
		// - to emit an event in the first place, the parser needs a token or eof
		// - if the parser has more events in the queue, it does not call the lexer but emits them directly
		// - thus, the blocked bit cannot be set to true
		// This has three effects:
		// 1. If the lexer (or parser) causes an error, it will be re-returned immediately without any reads from the AsyncBufRead, because blocked is false (so we do a empty-buffer-read) and the parser will re-emit any fatal error.
		// 2. If the lexer has internal state which causes it to emit another token even without reading further data, we give the parser a chance to convert that token to an event without awaiting on the source in between. This effectively avoids the caveat the PullParser has with blocking I/O (see the doc there for details, as well as the docs of Lexer::lex and Lexer::lex_bytes).
		// 3. If the parser has events buffered, they will be emitted without further reads from the source.
		if !self.blocked {
			let mut empty: &[u8] = &[];
			let (_, result) = Self::parse(&mut self.lexer, &mut self.parser, &mut empty, &mut self.blocked);
			if let Some(result) = result {
				return result
			}
		}
		loop {
			let mut buf = self.reader.fill_buf().await?;
			if buf.len() == 0 {
				return self.parser.parse(&mut EofLexerAdapter{lexer: &mut self.lexer});
			}
			let (nread, result) = Self::parse(&mut self.lexer, &mut self.parser, &mut buf, &mut self.blocked);
			self.reader.consume(nread);
			if let Some(result) = result {
				return result
			}
		}
	}
}
