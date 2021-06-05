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
*/
#[allow(unused_imports)]
use std::io;

mod error;
pub mod lexer;
pub mod parser;
mod bufq;
pub mod strings;
mod selectors;

#[cfg(test)]
mod tests;

#[doc(inline)]
pub use error::{Error, Result};
#[doc(inline)]
pub use lexer::{Lexer, LexerOptions, DecodingReader};
#[doc(inline)]
pub use parser::{QName, Parser, Event, LexerAdapter, XMLVersion, XMLNS_XML};
#[doc(inline)]
pub use bufq::BufferQueue;
pub use strings::{NCName, Name, NCNameStr, NameStr, CData, CDataStr};

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
	token_source: LexerAdapter<DecodingReader<BufferQueue<'x>>>,
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
		FeedParser{
			token_source: LexerAdapter::new(
				Lexer::new(),
				DecodingReader::new(BufferQueue::new()),
			),
			parser: Parser::new(),
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
		self.token_source.get_mut().get_mut().push(data);
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
		self.token_source.get_mut().get_mut().push_eof();
	}

	/// Return the amount of bytes which have not been read from the buffer
	/// yet.
	///
	/// This may not reflect the amount of memory used by the buffer
	/// accurately, as memory is only released when an entire chunk (as fed
	/// to `feed()`) has been processed (and only if that chunk is owned by
	/// the parser).
	pub fn buffered(&self) -> usize {
		self.token_source.get_ref().get_ref().len()
	}

	/// Return a reference to the internal buffer BufferQueue
	///
	/// This can be used to force dropping of all memory in case of error
	/// conditions.
	pub fn get_buffer_mut(&mut self) -> &mut BufferQueue<'x> {
		self.token_source.get_mut().get_mut()
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
pub struct PullParser<T: io::Read + Sized> {
	token_source: LexerAdapter<DecodingReader<T>>,
	parser: Parser,
}

impl<T: io::Read + Sized> PullParser<T> {
	/// Create a new PullParser, wrapping the given reader.
	///
	/// **Note:** It is highly recommended to wrap a common reader into
	/// [`std::io::BufReader`] as the implementation will do lots of small
	/// `read()` calls. Those would be terribly inefficient without buffering.
	pub fn new(r: T) -> Self {
		PullParser{
			token_source: LexerAdapter::new(
				Lexer::new(),
				DecodingReader::new(r),
			),
			parser: Parser::new(),
		}
	}
}

impl<T: io::Read + Sized> EventRead for PullParser<T> {
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
