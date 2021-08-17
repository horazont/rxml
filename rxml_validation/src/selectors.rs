/*!
# Constants and Utilities for matching ranges of codepoints

The contents of this module are implementation details of `rxml`,
`rxml_validation` and `rxml_proc` and should not be relied upon.
*/
use std::fmt;


/**
# Predicate trait for matching chars
*/
pub trait CharSelector {
	/// Return true if the given char is selected by the selector
	fn select(&self, c: char) -> bool;
}

/// Selects all chars
#[derive(Debug, Copy, Clone)]
pub struct AllChars();

impl CharSelector for char {
	fn select(&self, c: char) -> bool {
		*self == c
	}
}

impl CharSelector for &'_ [char] {
	fn select(&self, c: char) -> bool {
		for r in self.iter() {
			if *r == c {
				return true;
			}
		}
		false
	}
}

impl CharSelector for AllChars {
	fn select(&self, _c: char) -> bool {
		return true;
	}
}


// start to end (incl., because some of our edge points are not valid chars
// in rust)
/// Selects all chars from a range (including both ends)
pub struct CodepointRange(pub char, pub char);


/// Valid codepoints for character data (XML 1.0 § 2.2)
pub const VALID_XML_CDATA_RANGES: &'static [CodepointRange] = &[
	CodepointRange('\x09', '\x0a'),
	CodepointRange('\x0d', '\x0d'),
	CodepointRange('\u{0020}', '\u{d7ff}'),
	CodepointRange('\u{e000}', '\u{fffd}'),
	CodepointRange('\u{10000}', '\u{10ffff}'),
];


/// Invalid codepoints for character data (XML 1.0 § 2.2)
///
/// Note that values which are not valid Rust characters are not included in
/// these ranges.
pub const INVALID_XML_CDATA_RANGES: &'static [CodepointRange] = &[
	CodepointRange('\x00', '\x08'),
	CodepointRange('\x0b', '\x0c'),
	CodepointRange('\x0e', '\x1f'),
	CodepointRange('\u{fffe}', '\u{ffff}'),
];


const VALID_XML_NAME_START_RANGES: &'static [CodepointRange] = &[
	CodepointRange(':', ':'),
	CodepointRange('A', 'Z'),
	CodepointRange('_', '_'),
	CodepointRange('a', 'z'),
	CodepointRange('\u{c0}', '\u{d6}'),
	CodepointRange('\u{d8}', '\u{f6}'),
	CodepointRange('\u{f8}', '\u{2ff}'),
	CodepointRange('\u{370}', '\u{37d}'),
	CodepointRange('\u{37f}', '\u{1fff}'),
	CodepointRange('\u{200c}', '\u{200d}'),
	CodepointRange('\u{2070}', '\u{218f}'),
	CodepointRange('\u{2c00}', '\u{2fef}'),
	CodepointRange('\u{3001}', '\u{d7ff}'),
	CodepointRange('\u{f900}', '\u{fdcf}'),
	CodepointRange('\u{10000}', '\u{effff}'),
];


const VALID_XML_NAME_RANGES: &'static [CodepointRange] = &[
	CodepointRange(':', ':'),
	CodepointRange('-', '-'),
	CodepointRange('.', '.'),
	CodepointRange('A', 'Z'),
	CodepointRange('_', '_'),
	CodepointRange('0', '9'),
	CodepointRange('a', 'z'),
	CodepointRange('\u{b7}', '\u{b7}'),
	CodepointRange('\u{c0}', '\u{d6}'),
	CodepointRange('\u{d8}', '\u{f6}'),
	CodepointRange('\u{f8}', '\u{2ff}'),
	CodepointRange('\u{300}', '\u{36f}'),
	CodepointRange('\u{370}', '\u{37d}'),
	CodepointRange('\u{37f}', '\u{1fff}'),
	CodepointRange('\u{200c}', '\u{200d}'),
	CodepointRange('\u{203f}', '\u{2040}'),
	CodepointRange('\u{2070}', '\u{218f}'),
	CodepointRange('\u{2c00}', '\u{2fef}'),
	CodepointRange('\u{3001}', '\u{d7ff}'),
	CodepointRange('\u{f900}', '\u{fdcf}'),
	CodepointRange('\u{10000}', '\u{effff}'),
];

impl CodepointRange {
	pub fn contains(&self, c: char) -> bool {
		return (self.0 <= c) && (c <= self.1)
	}
}

/// Selects all chars from any of the contained ranges
#[derive(Copy)]
pub struct CodepointRanges(pub &'static [CodepointRange]);

/// Valid non-first characters for an XML Name (XML 1.0 § 2.3 \[4a\])
pub static CLASS_XML_NAME: CodepointRanges = CodepointRanges(VALID_XML_NAME_RANGES);

/// Valid first characters for an XML Name (XML 1.0 § 2.3 \[4\])
pub static CLASS_XML_NAMESTART: CodepointRanges = CodepointRanges(VALID_XML_NAME_START_RANGES);

/// See [`INVALID_XML_CDATA_RANGES`]
pub static CLASS_XML_NONCHAR: CodepointRanges = CodepointRanges(INVALID_XML_CDATA_RANGES);

impl CharSelector for CodepointRange {
	fn select(&self, c: char) -> bool {
		self.contains(c)
	}
}

impl CharSelector for CodepointRanges {
	fn select(&self, c: char) -> bool {
		contained_in_ranges(c, self.0)
	}
}

pub fn contained_in_ranges(c: char, rs: &[CodepointRange]) -> bool {
	for r in rs.iter() {
		if r.contains(c) {
			return true;
		}
	}
	false
}

impl fmt::Debug for CodepointRanges {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		write!(f, "CodepointRanges(<{} ranges>)", self.0.len())
	}
}

impl Clone for CodepointRanges {
	fn clone(&self) -> Self {
		CodepointRanges(self.0)
	}
}

impl PartialEq for CodepointRanges {
	fn eq(&self, other: &CodepointRanges) -> bool {
		std::ptr::eq(&self.0, &other.0)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cdata_inclusion_and_exclusion_are_equivalent() {
		let excluder = CodepointRanges(INVALID_XML_CDATA_RANGES);
		let includer = CodepointRanges(VALID_XML_CDATA_RANGES);
		for cp in 0x0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(cp) {
				if !includer.select(ch) != excluder.select(ch) {
					panic!("INVALID_XML_CDATA_RANGES and VALID_XML_CDATA_RANGES have different opinions about U+{:x}", cp)
				}
			}
		}
	}
}
