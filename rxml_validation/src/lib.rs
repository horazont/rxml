/*!
# Validator functions for XML-related strings

This is a supplementary crate for [`rxml`](https://docs.rs/rxml). It is
factored out of the main crate to support
[`rxml_proc`](https://docs.rs/rxml_proc), a crate of macros which allow
compile-time validation and typing of XML strings.
*/
use std::fmt;

pub mod selectors;

use selectors::CharSelector;

/**
Error condition from validating an XML string.
*/
#[derive(Debug, Clone)]
pub enum Error {
	/// A Name or NCName was empty.
	EmptyName,
	/// An invalid character was encountered.
	///
	/// This variant contains the character as data.
	InvalidChar(char),
}

impl fmt::Display for Error {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		match self {
			Self::EmptyName => f.write_str("Name and NCName must not be empty"),
			Self::InvalidChar(c) => write!(f, "character U+{:04x} is not allowed", *c as u32),
		}
	}
}

impl std::error::Error for Error {}

/**
Check whether a str is a valid XML 1.0 Name

# Example

```rust
use rxml_validation::{validate_name, Error};

assert!(validate_name("foobar").is_ok());
assert!(validate_name("foo:bar").is_ok());
assert!(matches!(validate_name("foo bar"), Err(Error::InvalidChar(' '))));
assert!(matches!(validate_name(""), Err(Error::EmptyName)));
*/
pub fn validate_name(s: &str) -> Result<(), Error> {
	let mut chars = s.chars();
	match chars.next() {
		// must have at least one char
		None => return Err(Error::EmptyName),
		Some(c) => if !selectors::CLASS_XML_NAMESTART.select(c) {
			return Err(Error::InvalidChar(c))
		}
	}
	for ch in chars {
		if !selectors::CLASS_XML_NAME.select(ch) {
			return Err(Error::InvalidChar(ch))
		}
	}
	Ok(())
}


/**
Check whether a str is a valid XML 1.0 Name, without colons.

# Example

```rust
use rxml_validation::{validate_ncname, Error};

assert!(validate_ncname("foobar").is_ok());
assert!(matches!(validate_ncname("foo:bar"), Err(Error::InvalidChar(':'))));
assert!(matches!(validate_ncname(""), Err(Error::EmptyName)));
*/
pub fn validate_ncname(s: &str) -> Result<(), Error> {
	let mut chars = s.chars();
	match chars.next() {
		// must have at least one char
		None => return Err(Error::EmptyName),
		Some(c) => if !selectors::CLASS_XML_NAMESTART.select(c) || c == ':' {
			return Err(Error::InvalidChar(c))
		}
	}
	for ch in chars {
		if !selectors::CLASS_XML_NAME.select(ch) || ch == ':' {
			return Err(Error::InvalidChar(ch))
		}
	}
	Ok(())
}

/**
Check whether a str is valid XML 1.0 CData

# Example

```rust
use rxml_validation::{validate_cdata, Error};

assert!(validate_cdata("foo bar baz <fnord!>").is_ok());
assert!(matches!(validate_cdata("\x01"), Err(Error::InvalidChar('\x01'))));
*/
pub fn validate_cdata(s: &str) -> Result<(), Error> {
	for ch in s.chars() {
		if selectors::CLASS_XML_NONCHAR.select(ch) {
			return Err(Error::InvalidChar(ch))
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_cdata_smoketest() {
		assert!(validate_cdata("foo bar baz http://<xyz>").is_ok());
		assert!(validate_cdata("\u{ffff}").is_err());
	}

	#[test]
	fn test_name_smoketest() {
		assert!(validate_name("foobar").is_ok());
		assert!(validate_name("foo:bar").is_ok());
		assert!(validate_name("").is_err());
		assert!(validate_name("foo bar baz http://<xyz>").is_err());
		assert!(validate_name("\u{ffff}").is_err());
	}

	#[test]
	fn test_ncname_smoketest() {
		assert!(validate_ncname("foobar").is_ok());
		assert!(validate_ncname("foo:bar").is_err());
		assert!(validate_ncname("").is_err());
		assert!(validate_ncname("foo bar baz http://<xyz>").is_err());
		assert!(validate_ncname("\u{ffff}").is_err());
	}
}
