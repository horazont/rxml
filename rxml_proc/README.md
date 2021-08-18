# `rxml_proc` — Compile-time validation of CData, Name and NCName strings

This crate is supplementary to the `rxml` crate. It provides three macros (`xml_cdata!`, `xml_name!` and `xml_ncname!`) which convert a normal `&str` into the corresponding `rxml` string type for strong typing of XML string flavors.

[![crate badge](https://img.shields.io/crates/v/rxml_proc.svg)](https://crates.io/crates/rxml_proc) [![docs badge](https://docs.rs/rxml_proc/badge.svg)](https://docs.rs/rxml_proc/)

Please see the [rxml](https://crates.io/crates/rxml) crate for more information.

## Example

```rust
use rxml::NCNameStr;
use rxml_proc::xml_ncname;

const XML_PREFIX: &'static NCNameStr = xml_ncname!("xml");
```
