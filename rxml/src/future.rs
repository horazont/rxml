use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::AsyncBufRead;

#[cfg(feature = "stream")]
use futures_core::stream::Stream;

use crate::driver::PushDriver;
use crate::lexer::{Lexer, LexerOptions};
use crate::parser::{Parse, Parser};
use crate::{Error, Result};

use pin_project_lite::pin_project;

pin_project! {
	pub struct ReadEvent<T: ?Sized>{
		#[pin]
		inner: T,
	}
}

impl<T: AsyncEventRead + Unpin> Future for ReadEvent<T> {
	type Output = Result<Option<T::Output>>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.project().inner.poll_read(cx)
	}
}

pin_project! {
	pub struct ReadAll<T: ?Sized, F> {
		cb: F,
		#[pin]
		inner: T,
	}
}

impl<T: AsyncEventRead + Unpin, F: FnMut(T::Output) -> () + Send> Future for ReadAll<T, F> {
	type Output = Result<()>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
		let mut this = self.project();
		loop {
			match this.inner.as_mut().poll_read(cx) {
				Poll::Ready(Ok(Some(ev))) => {
					(this.cb)(ev);
				}
				Poll::Ready(Ok(None)) => return Poll::Ready(Ok(())),
				Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
				Poll::Pending => return Poll::Pending,
			}
		}
	}
}

/**
Asynchronous source of individual XML events

This trait is implemented by the different parser frontends. It is analogous to the [`tokio::io::AsyncRead`] trait, but for [`ResolvedEvent`]s instead of bytes.

Usually, one interacts with this trait through the helpers available in [`AsyncEventReadExt`]

   [`ResolvedEvent`]: crate::ResolvedEvent
*/
pub trait AsyncEventRead {
	type Output;

	/// Poll for a single event from the parser.
	fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<Option<Self::Output>>>;
}

impl<T: AsyncEventRead + Unpin + ?Sized> AsyncEventRead for &mut T {
	type Output = T::Output;

	fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<Option<Self::Output>>> {
		let this: &mut &mut T = Pin::into_inner(self);
		let this: &mut T = *this;
		let this = Pin::new(this);
		this.poll_read(cx)
	}
}

#[cfg(feature = "stream")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "stream", feature = "async"))))]
impl<T: AsyncBufRead, P: Parse> Stream for AsyncDriver<T, P> {
	type Item = Result<P::Output>;

	fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		match self.poll_read(cx) {
			Poll::Pending => Poll::Pending,
			Poll::Ready(Ok(Some(v))) => Poll::Ready(Some(Ok(v))),
			Poll::Ready(Ok(None)) => Poll::Ready(None),
			Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
		}
	}
}

/**
Helper trait for asynchronous sources of individual XML events

This helper trait is automatically implemented for all [`AsyncEventRead`].
*/
pub trait AsyncEventReadExt: AsyncEventRead {
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
	/// async fn read(&mut self) -> Result<Option<ResolvedEvent>>;
	/// ```
	fn read(&mut self) -> ReadEvent<&mut Self> {
		ReadEvent { inner: self }
	}

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
	///     async fn read_all<F>(&mut self, mut cb: F) -> Result<()>
	///            where F: FnMut(ResolvedEvent) -> () + Send
	/// ```
	fn read_all<F>(&mut self, cb: F) -> ReadAll<&mut Self, F> {
		ReadAll { inner: self, cb }
	}
}

impl<T: AsyncEventRead> AsyncEventReadExt for T {}

pin_project! {
	/**
	# Asynchronous driver for parsers

	This is a generic asynchronous driver for objects implementing the
	[`Parse`] trait.

	In general, it is advised to use the [`AsyncParser`] alias which
	specializes this struct for use with the default [`Parser`].
	*/
	#[project = AsyncDriverProj]
	pub struct AsyncDriver<T, P: Parse>{
		#[pin]
		inner: T,
		driver: PushDriver<P>,
	}
}

impl<T: AsyncBufRead, P: Parse + Default> AsyncDriver<T, P> {
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

impl<T: AsyncBufRead, P: Parse> AsyncDriver<T, P> {
	/// Create a fully customized parser from a lexer and a parser component.
	pub fn wrap(inner: T, lexer: Lexer, parser: P) -> Self {
		Self {
			inner,
			driver: PushDriver::wrap(lexer, parser),
		}
	}

	/// Decompose the AsyncDriver into its parts
	pub fn into_inner(self) -> (T, Lexer, P) {
		let (lexer, parser) = self.driver.into_inner();
		(self.inner, lexer, parser)
	}

	/// Access the inner AsyncBufRead
	pub fn get_inner(&self) -> &T {
		&self.inner
	}

	/// Access the inner AsyncBufRead, mutably
	pub fn get_inner_mut(&mut self) -> &mut T {
		&mut self.inner
	}

	/// Access the lexer
	pub fn get_lexer(&self) -> &Lexer {
		self.driver.get_lexer()
	}

	/// Access the lexer, mutably
	pub fn get_lexer_mut(&mut self) -> &mut Lexer {
		self.driver.get_lexer_mut()
	}

	/// Access the parser
	pub fn get_parser(&self) -> &P {
		self.driver.get_parser()
	}

	/// Access the parser, mutably
	pub fn get_parser_mut(&mut self) -> &mut P {
		self.driver.get_parser_mut()
	}

	/// Release temporary buffers and other ephemeral allocations.
	///
	/// This is sensible to call when it is expected that no more data will be
	/// processed by the parser for a while and the memory is better used
	/// elsewhere.
	#[inline(always)]
	pub fn release_temporaries(&mut self) {
		self.driver.release_temporaries();
	}
}

impl<T, P: Parse> AsyncDriver<T, P> {
	fn parse_step(
		driver: &mut PushDriver<P>,
		buf: &mut &[u8],
		may_eof: bool,
	) -> (usize, Poll<Result<Option<P::Output>>>) {
		let old_len = buf.len();
		// need to guard eof with the buf len here, because we only know that we are actually at eof by the fact that we see an empty buffer.
		let result = driver.parse(buf, may_eof && buf.len() == 0);
		let new_len = buf.len();
		assert!(new_len <= old_len);
		let read = old_len - new_len;
		match result {
			Ok(v) => (read, Poll::Ready(Ok(v))),
			Err(Error::IO(ioerr)) if ioerr.kind() == io::ErrorKind::WouldBlock => {
				(read, Poll::Pending)
			}
			Err(e) => (read, Poll::Ready(Err(e))),
		}
	}
}

impl<T: AsyncBufRead, P: Parse> AsyncEventRead for AsyncDriver<T, P> {
	type Output = P::Output;

	fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<Option<P::Output>>> {
		let mut this = self.project();
		loop {
			let mut buf = match this.inner.as_mut().poll_fill_buf(cx) {
				Poll::Pending => {
					// a.k.a. WouldBlock
					// we always try an empty read here because the lexer needs that
					return Self::parse_step(this.driver, &mut &[][..], false).1;
				}
				Poll::Ready(Ok(buf)) => buf,
				Poll::Ready(Err(e)) => return Poll::Ready(Err(e.into())),
			};
			let (consumed, result) = Self::parse_step(this.driver, &mut buf, true);
			this.inner.as_mut().consume(consumed);
			match result {
				// if we get a pending here, we need to ask the source for more data!
				Poll::Pending => continue,
				Poll::Ready(v) => return Poll::Ready(v),
			}
		}
	}
}

/**
Tokio-compatible asynchronous parser

The [`AsyncParser`] allows parsing XML documents from a [`tokio::io::AsyncBufRead`], asynchronously. It operates similarly as the [`PullParser`] does, but instead of blocking the task, it will yield control to other tasks if the backend is not able to supply data immediately.

This is a type alias around a [`AsyncDriver`] and documentation for the API is
found there.

Interaction with a `AsyncParser` should happen exclusively via the [`AsyncEventReadExt`] trait.

## Example

The example is a bit pointless because it does not really demonstrate the asynchronicity.

```
use rxml::{AsyncParser, Error, ResolvedEvent, XmlVersion, AsyncEventReadExt};
use tokio::io::AsyncRead;
# tokio_test::block_on(async {
let mut doc = &b"<?xml version='1.0'?><hello>World!</hello>"[..];
// this converts the doc into an tokio::io::AsyncRead
let mut pp = AsyncParser::new(&mut doc);
// we expect the first event to be the XML declaration
let ev = pp.read().await;
assert!(matches!(ev.unwrap().unwrap(), ResolvedEvent::XMLDeclaration(_, XmlVersion::V1_0)));
# })
```

## Parsing without namespace expansion

To parse an XML document without namespace expansion in blocking mode,
one can use the [`AsyncDriver`] with a [`RawParser`]. Note the caveats in the
[`RawParser`] documentation before using it!

   [`RawParser`]: crate::parser::RawParser
   [`PullParser`]: crate::PullParser
*/
pub type AsyncParser<T> = AsyncDriver<T, Parser>;
