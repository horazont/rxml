# `rxml` Workspace

This workspace hosts the `rxml` family of crates. It offers low-complexity parsing of a restricted subset of XML 1.0.

- [`rxml`](rxml/) [![rxml crate info](https://img.shields.io/crates/v/rxml.svg)](https://crates.io/crates/rxml) [![rxml docs badge](https://docs.rs/rxml/badge.svg)](https://docs.rs/rxml/): Non-blocking and streaming XML parser.

- [`rxml_proc`](rxml_proc/) [![rxml_proc crate info](https://img.shields.io/crates/v/rxml_proc.svg)](https://crates.io/crates/rxml_proc) [![rxml_proc docs](https://docs.rs/rxml_proc/badge.svg)](https://docs.rs/rxml_proc/): Macros for compile-time validation of string constants against XML string types.

- [`rxml_validation`](rxml_validation/) [![rxml_validation crate info](https://img.shields.io/crates/v/rxml_validation.svg)](https://crates.io/crates/rxml_proc) [![rxml_validation docs](https://docs.rs/rxml_validation/badge.svg)](https://docs.rs/rxml_validation/): Utilities shared between [`rxml`] and [`rxml_proc`], to reduce the amount of code pulled into the compiler by `rxml_proc`.
