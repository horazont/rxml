pub trait ByteSelector<'x> {
	fn find_any_first_in<'b>(&'x self, haystack: &'b [u8]) -> Option<(usize, u8)>;
}

/// Select bytes which may be part of an XML name.
///
/// As this operates on bytes and does not decode UTF-8, it is only an
/// approximation. Full validation of XML name requirements need to be done at
/// a later stage.
pub struct XMLNameBytePreselector();
pub struct XMLNonNameBytePreselector();

#[inline]
pub fn byte_maybe_xml_name(v: u8) -> bool {
	match v {
		b':' => true,
		b'_' => true,
		b'A'..=b'Z' => true,
		b'a'..=b'z' => true,
		// name only
		b'-' => true,
		b'.' => true,
		b'0'..=b'9' => true,
		// end of name only
		// unicode maybe part of a name
		0x80u8..=0xffu8 => true,
		_ => false,
	}
}

#[inline]
pub fn byte_maybe_xml_namestart(v: u8) -> bool {
	match v {
		b':' => true,
		b'_' => true,
		b'A'..=b'Z' => true,
		b'a'..=b'z' => true,
		// unicode maybe part of a name
		0x80u8..=0xffu8 => true,
		_ => false,
	}
}

impl ByteSelector<'_> for XMLNameBytePreselector {
	#[inline]
	fn find_any_first_in<'b>(&self, haystack: &'b [u8]) -> Option<(usize, u8)> {
		for (i, b) in haystack.iter().enumerate() {
			let b = *b;
			if byte_maybe_xml_name(b) {
				return Some((i, b))
			}
		}
		None
	}
}

impl ByteSelector<'_> for XMLNonNameBytePreselector {
	#[inline]
	fn find_any_first_in<'b>(&self, haystack: &'b [u8]) -> Option<(usize, u8)> {
		for (i, b) in haystack.iter().enumerate() {
			let b = *b;
			if !byte_maybe_xml_name(b) {
				return Some((i, b))
			}
		}
		None
	}
}

pub struct InvertDelimiters<'x>(pub &'x [u8]);

impl<'x> ByteSelector<'x> for InvertDelimiters<'x> {
	#[inline]
	fn find_any_first_in<'b>(&'x self, haystack: &'b [u8]) -> Option<(usize, u8)> {
		let delimiters = self.0;
		for (i, b) in haystack.iter().enumerate() {
			let mut matches_any = false;
			for d in delimiters.iter() {
				if *d == *b {
					matches_any = true;
					break;
				}
			}
			if !matches_any {
				return Some((i, *b))
			}
		}
		None
	}
}

impl<'x> ByteSelector<'x> for &'x [u8] {
	#[inline]
	fn find_any_first_in<'b>(&'x self, haystack: &'b [u8]) -> Option<(usize, u8)> {
		for (i, b) in haystack.iter().enumerate() {
			for d in self.iter() {
				if *d == *b {
					return Some((i, *d))
				}
			}
		}
		None
	}
}

impl<'x> ByteSelector<'x> for u8 {
	#[inline]
	fn find_any_first_in<'b>(&'x self, haystack: &'b [u8]) -> Option<(usize, u8)> {
		for (i, b) in haystack.iter().enumerate() {
			if *self == *b {
				return Some((i, *self))
			}
		}
		None
	}
}


const DELIM_ELEMENT_START: u8 = '<' as u8;
const DELIM_REF_START: u8 = '&' as u8;
pub const DELIM_TEXT_STATE_EXIT: &'static [u8] = &[DELIM_ELEMENT_START, DELIM_REF_START];

pub const CLASS_XML_SPACES: &'static [u8] = b" \t\r\n";
pub const CLASS_XML_DECIMAL_DIGITS: &'static [u8] = b"0123456789";
pub const CLASS_XML_HEXADECIMAL_DIGITS: &'static [u8] = b"0123456789abcdefABCDEF";


// start to end (incl., because some of our edge points are not valid chars
// in rust)
pub struct CodepointRange(char, char);

// XML 1.0 § 2.2
pub const VALID_XML_CDATA_RANGES: &'static [CodepointRange] = &[
	CodepointRange('\x09', '\x0a'),
	CodepointRange('\x0d', '\x0d'),
	CodepointRange('\u{0020}', '\u{d7ff}'),
	CodepointRange('\u{e000}', '\u{fffd}'),
	CodepointRange('\u{10000}', '\u{10ffff}'),
];


// XML 1.0 § 2.3 [4]
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


// XML 1.0 § 2.3 [4a]
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

pub fn contained_in_ranges(c: char, rs: &[CodepointRange]) -> bool {
	for r in rs.iter() {
		if r.contains(c) {
			return true;
		}
	}
	false
}
