/*!
# Restricted XML parsing and encoding

This crate provides "restricted" parsing and encoding of XML 1.0 documents
with namespacing.

## Features (some call them restrictions)

* No external resources
* No custom entities
* No DTD whatsoever
* No processing instructions
* No comments
* UTF-8 only
* Namespacing-well-formedness enforced
* XML 1.0 only
* Streamed parsing (parser emits a subset of SAX events)
* Streamed encoding
* Parser can be driven push- and pull-based
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

## High-level parser usage

### Push-based usage

The [`FeedParser`] allows to push bits of XML into the parser as they arrive
in the application and process the resulting [`ResolvedEvent`]s as they
happen.

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

mod bufq;
mod context;
mod driver;
mod errctx;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod strings;
pub mod writer;

#[cfg(test)]
mod tests;

#[doc(inline)]
pub use bufq::BufferQueue;
pub use context::Context;
#[doc(inline)]
pub use driver::{as_eof_flag, EventRead, FeedParser, PullDriver, PullParser, PushDriver};
#[doc(inline)]
pub use error::{Error, Result};
#[doc(inline)]
pub use lexer::{Lexer, LexerOptions};
#[doc(inline)]
pub use parser::{
	LexerAdapter, NamespaceResolver, Parse, Parser, RawEvent, RawParser, RawQName, ResolvedEvent,
	ResolvedQName, XMLVersion, XMLNS_XML,
	WithContext,
};
pub use strings::{CData, CDataStr, NCName, NCNameStr, Name, NameStr};
#[doc(inline)]
pub use writer::{Encoder, Item};

#[cfg(feature = "macros")]
#[doc(inline)]
#[cfg_attr(docsrs, doc(cfg(feature = "macros")))]
pub use rxml_proc::{xml_cdata, xml_name, xml_ncname};

#[cfg(feature = "async")]
mod future;

#[cfg(feature = "async")]
#[doc(inline)]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use future::{AsyncDriver, AsyncEventRead, AsyncEventReadExt, AsyncParser};

/// Package version
pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

/// Compatibility alias, use [`ResolvedEvent`] directly instead.
#[deprecated(since = "0.7.0", note = "type was renamed to ResolvedEvent")]
pub type Event = ResolvedEvent;
/// Compatibility alias, use [`ResolvedQName`] directly instead.
#[deprecated(since = "0.7.0", note = "type was renamed to ResolvedQName")]
pub type QName = ResolvedQName;
