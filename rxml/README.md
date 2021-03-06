# `rxml` — Restricted, minimalistic XML 1.0 parser

This crate provides "restricted" parsing of XML 1.0 documents with
namespacing.

[![crate badge](https://img.shields.io/crates/v/rxml.svg)](https://crates.io/crates/rxml) [![docs badge](https://docs.rs/rxml/badge.svg)](https://docs.rs/rxml/)

**Warning:** This crate is alpha-quality! That means you should probably not
yet put it in a network-facing position. CVE numbers may or may not be
allocated for security issues in releases where this text is present.

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
* Tokio-based asynchronicity supported via the `async` feature and `AsyncParser`.

## Example

```rust
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

## Related crates

- [`rxml_proc`](https://crates.io/crates/rxml_proc) offers macros for compile-time conversion of strings to strongly-typed XML-specific str subtypes.
