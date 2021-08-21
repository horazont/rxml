#[cfg(test)]
use rxml_validation::selectors::{
	CodepointRange,
	CodepointRanges,
	CLASS_XML_NAMESTART,
};

pub trait ByteSelect {
	fn select(&self, b: u8) -> bool;
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct ByteRange(u8, u8);

impl ByteSelect for ByteRange {
	fn select(&self, b: u8) -> bool {
		self.0 <= b && b <= self.1
	}
}

impl ByteSelect for u8 {
	fn select(&self, b: u8) -> bool {
		b == *self
	}
}

impl ByteSelect for &'_ [u8] {
	fn select(&self, b: u8) -> bool {
		for r in *self {
			if *r == b {
				return true;
			}
		}
		false
	}
}

pub struct AnyByte();

impl ByteSelect for AnyByte {
	fn select(&self, _b: u8) -> bool {
		true
	}
}

impl ByteSelect for &'_ [ByteRange] {
	fn select(&self, b: u8) -> bool {
		for r in *self {
			if r.select(b) {
				return true;
			}
		}
		false
	}
}

pub static CLASS_XML_NAMESTART_BYTE: &'static [ByteRange] = &[
	ByteRange(b':', b':'),
	ByteRange(b'A', b'Z'),
	ByteRange(b'_', b'_'),
	ByteRange(b'a', b'z'),
	// and now essentially all utf8 start bytes
	ByteRange(b'\xc3', b'\xf7'),
];

pub static CLASS_XML_NAME_BYTE: &'static [ByteRange] = &[
	ByteRange(b':', b':'),
	ByteRange(b'-', b'-'),
	ByteRange(b'.', b'.'),
	ByteRange(b'A', b'Z'),
	ByteRange(b'_', b'_'),
	ByteRange(b'0', b'9'),
	ByteRange(b'a', b'z'),
	ByteRange(b'\x80', b'\xff'),
];

pub static CLASS_XML_MAY_NONCHAR_BYTE: &'static [ByteRange] = &[
	ByteRange(b'\x00', b'\x08'),
	ByteRange(b'\x0b', b'\x0c'),
	ByteRange(b'\x0e', b'\x1f'),
];

/// Valid bytes for XML character data minus delimiters (XML 1.0 § 2.4 [14])
///
/// This is like [`VALID_XML_CDATA_RANGES`], but the following chars are excluded:
///
/// - `'\r'`, because that gets folded into a line feed (`\n`) on input
/// - `'&'`, because that may start an entity or character reference
/// - `'<'`, because that may start an element or CDATA section
/// - `']'`, because that may end a CDATA section and the sequence `]]>` is not allowed verbatimly in character data in XML documents
pub static CLASS_XML_TEXT_DELIMITED_BYTE: &'static [ByteRange] = &[
	ByteRange(b'\x09', b'\x0a'),
	ByteRange(b'\x20', b'\x25'), // excludes &
	ByteRange(b'\x27', b'\x3b'), // excludes <
	ByteRange(b'\x3d', b'\x5c'), // excludes ]
	ByteRange(b'\x5e', b'\x7f'),
	ByteRange(b'\x80', b'\xff'),
];

// XML 1.0 § 2.4 [14]
pub static CLASS_XML_CDATA_CDATASECTION_DELIMITED_BYTE: &'static [ByteRange] = &[
	ByteRange(b'\x09', b'\x0a'),
	// excluding CR as that gets folded to LF
	ByteRange(b'\x20', b'\x5c'), // excludes ]
	ByteRange(b'\x5e', b'\x7f'),
	ByteRange(b'\x80', b'\xff'),
];

/// XML whitespace
pub static CLASS_XML_SPACE_BYTE: &'static [u8] = b" \t\r\n";


// XML 1.0 § 2.3 [10]
pub const CLASS_XML_CDATA_ATT_APOS_DELIMITED_BYTE: &'static [ByteRange] = &[
	// exclude all whitespace except normal space because those get converted into spaces
	ByteRange(b'\x20', b'\x25'), // excludes &, '
	ByteRange(b'\x28', b'\x3b'), // excludes <
	ByteRange(b'\x3d', b'\xff'),
];


// XML 1.0 § 2.3 [10]
pub const CLASS_XML_CDATA_ATT_QUOT_DELIMITED_BYTE: &'static [ByteRange] = &[
	// exclude all whitespace except normal space because those get converted into spaces
	ByteRange(b'\x20', b'\x21'), // excludes "
	ByteRange(b'\x23', b'\x25'), // excludes &
	ByteRange(b'\x27', b'\x3b'), // excludes <
	ByteRange(b'\x3d', b'\xff'),
];

/// Valid XML decimal characters (for character references)
pub static CLASS_XML_DECIMAL_DIGIT_BYTE: ByteRange = ByteRange(b'0', b'9');

/// Valid XML hexadecimal characters (for character references)
pub static CLASS_XML_HEXADECIMAL_DIGIT_BYTE: &'static [ByteRange] = &[
	CLASS_XML_DECIMAL_DIGIT_BYTE,
	ByteRange(b'a', b'f'),
	ByteRange(b'A', b'F'),
];


/// Valid codepoints for XML character data minus delimiters (XML 1.0 § 2.4 [14])
///
/// This is like [`VALID_XML_CDATA_RANGES`], but the following chars are excluded:
///
/// - `'\r'`, because that gets folded into a line feed (`\n`) on input
/// - `'&'`, because that may start an entity or character reference
/// - `'<'`, because that may start an element or CDATA section
/// - `']'`, because that may end a CDATA section and the sequence `]]>` is not allowed verbatimly in character data in XML documents
#[cfg(test)]
const VALID_XML_CDATA_RANGES_TEXT_DELIMITED: &'static [CodepointRange] = &[
	CodepointRange('\x09', '\x0a'),
	// excluding CR as that gets folded to LF
	CodepointRange('\u{0020}', '\u{0025}'), // excludes &
	CodepointRange('\u{0027}', '\u{003b}'), // excludes <
	CodepointRange('\u{003d}', '\u{005c}'), // excludes ]
	CodepointRange('\u{005e}', '\u{d7ff}'),
	CodepointRange('\u{e000}', '\u{fffd}'),
	CodepointRange('\u{10000}', '\u{10ffff}'),
];


// XML 1.0 § 2.3 [10]
#[cfg(test)]
const VALID_XML_CDATA_RANGES_ATT_APOS_DELIMITED: &'static [CodepointRange] = &[
	// exclude all whitespace except normal space because those get converted into spaces
	CodepointRange('\u{0020}', '\u{0025}'), // excludes &, '
	CodepointRange('\u{0028}', '\u{003b}'), // excludes <
	CodepointRange('\u{003d}', '\u{d7ff}'),
	CodepointRange('\u{e000}', '\u{fffd}'),
	CodepointRange('\u{10000}', '\u{10ffff}'),
];


// XML 1.0 § 2.3 [10]
#[cfg(test)]
const VALID_XML_CDATA_RANGES_ATT_QUOT_DELIMITED: &'static [CodepointRange] = &[
	// exclude all whitespace except normal space because those get converted into spaces
	CodepointRange('\u{0020}', '\u{0021}'), // excludes "
	CodepointRange('\u{0023}', '\u{0025}'), // excludes &
	CodepointRange('\u{0027}', '\u{003b}'), // excludes <
	CodepointRange('\u{003d}', '\u{d7ff}'),
	CodepointRange('\u{e000}', '\u{fffd}'),
	CodepointRange('\u{10000}', '\u{10ffff}'),
];


// XML 1.0 § 2.4 [14]
#[cfg(test)]
const VALID_XML_CDATA_RANGES_CDATASECTION_DELIMITED: &'static [CodepointRange] = &[
	CodepointRange('\x09', '\x0a'),
	// excluding CR as that gets folded to LF
	CodepointRange('\u{0020}', '\u{005c}'), // excludes ]
	CodepointRange('\u{005e}', '\u{d7ff}'),
	CodepointRange('\u{e000}', '\u{fffd}'),
	CodepointRange('\u{10000}', '\u{10ffff}'),
];

#[cfg(test)]
static CLASS_XML_CDATA_SECTION_CONTENTS_DELIMITED: CodepointRanges = CodepointRanges(VALID_XML_CDATA_RANGES_CDATASECTION_DELIMITED);


#[cfg(test)]
mod tests {
	use super::*;
	use rxml_validation::selectors::*;

	#[test]
	fn test_namestart_byte_range_is_superset_of_namestart_codepoint_range() {
		let mut buf = [0u8; 4];
		for cp in 0x0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(cp) {
				let s = ch.encode_utf8(&mut buf[..]);
				if CLASS_XML_NAMESTART.select(ch) && !CLASS_XML_NAMESTART_BYTE.select(s.as_bytes()[0]) {
					panic!("byte selector rejects byte 0x{:02x}, which is the start byte of U+{:04x}", s.as_bytes()[0], cp);
				}
			}
		}
	}

	#[test]
	fn test_namestart_byte_range_rejects_invalid_utf8_start_bytes() {
		assert!(!CLASS_XML_NAMESTART_BYTE.select(b'\xc0'));
		for b in 0x80..0xc2u8 {
			if CLASS_XML_NAMESTART_BYTE.select(b) {
				panic!("accepts byte 0x{:02x}, which is not a valid UTF-8 start byte", b);
			}
		}
		for b in 0xf8..0xffu8 {
			if CLASS_XML_NAMESTART_BYTE.select(b) {
				panic!("accepts byte 0x{:02x}, which is not a valid UTF-8 start byte", b);
			}
		}
	}

	#[test]
	fn test_nonchar_byte_range_is_superset_of_nonchar_codepoint_range() {
		let mut buf = [0u8; 4];
		for cp in 0x0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(cp) {
				let s = ch.encode_utf8(&mut buf[..]);
				if !CLASS_XML_NONCHAR.select(ch) {
					let mut ok = false;
					for b in s.as_bytes() {
						if !CLASS_XML_MAY_NONCHAR_BYTE.select(*b) {
							ok = true;
						}
					}
					if !ok {
						panic!("byte selector accepts all bytes of U+{:04x}", cp);
					}
				}
			}
		}
	}

	#[test]
	fn test_text_delimited_byte_range_is_superset_of_text_delimited_codepoint_range() {
		let class = &CodepointRanges(VALID_XML_CDATA_RANGES_TEXT_DELIMITED);
		let mut buf = [0u8; 4];
		for cp in 0x0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(cp) {
				let s = ch.encode_utf8(&mut buf[..]);
				for b in s.as_bytes() {
					if class.select(ch) && !CLASS_XML_TEXT_DELIMITED_BYTE.select(*b) {
						panic!("byte selector rejects byte 0x{:02x}, which is a utf-8 byte of U+{:04x}", *b, cp);
					}
				}
			}
		}
	}

	#[test]
	fn test_cdata_delimited_byte_range_is_superset_of_codepoint_range() {
		let mut buf = [0u8; 4];
		for cp in 0x0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(cp) {
				let s = ch.encode_utf8(&mut buf[..]);
				for b in s.as_bytes() {
					if CLASS_XML_CDATA_SECTION_CONTENTS_DELIMITED.select(ch) && !CLASS_XML_CDATA_CDATASECTION_DELIMITED_BYTE.select(*b) {
						panic!("byte selector rejects byte 0x{:02x}, which is a utf-8 byte of U+{:04x}", *b, cp);
					}
				}
			}
		}
	}

	#[test]
	fn test_name_byte_range_is_superset_of_codepoint_range() {
		let mut buf = [0u8; 4];
		for cp in 0x0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(cp) {
				let s = ch.encode_utf8(&mut buf[..]);
				for b in s.as_bytes() {
					if CLASS_XML_NAME.select(ch) && !CLASS_XML_NAME_BYTE.select(*b) {
						panic!("byte selector rejects byte 0x{:02x}, which is a utf-8 byte of U+{:04x}", *b, cp);
					}
				}
			}
		}
	}

	#[test]
	fn test_att_apos_delimited_range_is_superset_of_codepoint_range() {
		let class = CodepointRanges(VALID_XML_CDATA_RANGES_ATT_APOS_DELIMITED);
		let mut buf = [0u8; 4];
		for cp in 0x0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(cp) {
				let s = ch.encode_utf8(&mut buf[..]);
				for b in s.as_bytes() {
					if class.select(ch) && !CLASS_XML_CDATA_ATT_APOS_DELIMITED_BYTE.select(*b) {
						panic!("byte selector rejects byte 0x{:02x}, which is a utf-8 byte of U+{:04x}", *b, cp);
					}
				}
			}
		}
	}

	#[test]
	fn test_att_quot_delimited_range_is_superset_of_codepoint_range() {
		let class = CodepointRanges(VALID_XML_CDATA_RANGES_ATT_QUOT_DELIMITED);
		let mut buf = [0u8; 4];
		for cp in 0x0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(cp) {
				let s = ch.encode_utf8(&mut buf[..]);
				for b in s.as_bytes() {
					if class.select(ch) && !CLASS_XML_CDATA_ATT_QUOT_DELIMITED_BYTE.select(*b) {
						panic!("byte selector rejects byte 0x{:02x}, which is a utf-8 byte of U+{:04x}", *b, cp);
					}
				}
			}
		}
	}
}
