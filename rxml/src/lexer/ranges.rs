#[cfg(test)]
use rxml_validation::selectors::{CodepointRange, CodepointRanges};

pub trait ByteSelect {
	fn select(&self, b: u8) -> bool;
}

pub(crate) struct AnyByte();

impl ByteSelect for AnyByte {
	fn select(&self, _b: u8) -> bool {
		true
	}
}

impl ByteSelect for u8 {
	fn select(&self, b: u8) -> bool {
		*self == b
	}
}

impl<T: Fn(u8) -> bool> ByteSelect for T {
	fn select(&self, b: u8) -> bool {
		self(b)
	}
}

/// XML whitespace
pub fn is_space(b: u8) -> bool {
	b == b' ' || b == b'\n' || b == b'\t' || b == b'\r'
}

/// Bytes not valid for XML character data (XML 1.0 § 2.4 [14])
fn is_text_delimiter(b: u8) -> bool {
	// - `'\r'`, because that gets folded into a line feed (`\n`) on input
	// - `'&'`, because that may start an entity or character reference
	// - `'<'`, because that may start an element or CDATA section
	// - `']'`, because that may end a CDATA section and the sequence `]]>` is not allowed verbatimly in character data in XML documents
	// - < 0x09, 0x0b..0x20, because XML forbids ASCII control characters except whitespace
	b == b'\r' || b == b'&' || b == b'<' || b == b']' || b < 0x09 || (b > 0x0a && b < 0x20)
}

pub fn maybe_text(b: u8) -> bool {
	!is_text_delimiter(b)
}

// XML 1.0 § 2.4 [14]
fn is_cdata_content_delimiter(b: u8) -> bool {
	b == b'\r' || b == b']' || b < 0x09 || (b > 0x0a && b < 0x20)
}

pub fn maybe_cdata_content(b: u8) -> bool {
	!is_cdata_content_delimiter(b)
}

fn is_name_delimiter(b: u8) -> bool {
	if b == b':' || b == b'-' || b == b'.' || b == b'_' {
		return false;
	}
	if b >= b'a' && b <= b'z' {
		return false;
	}
	if b >= b'0' && b <= b'9' {
		return false;
	}
	if b >= b'A' && b <= b'Z' {
		return false;
	}
	if b >= 0x80 {
		return false;
	}
	true
}

pub fn maybe_name(b: u8) -> bool {
	!is_name_delimiter(b)
}

// XML 1.0 § 2.3 [10]
fn is_attval_apos_delimiter(b: u8) -> bool {
	// exclude all whitespace except normal space because those get converted into spaces
	b < 0x20 || b == b'&' || b == b'\'' || b == b'<'
}

pub fn maybe_attval_apos(b: u8) -> bool {
	!is_attval_apos_delimiter(b)
}

// XML 1.0 § 2.3 [10]
fn is_attval_quot_delimiter(b: u8) -> bool {
	// exclude all whitespace except normal space because those get converted into spaces
	b < 0x20 || b == b'&' || b == b'"' || b == b'<'
}

pub fn maybe_attval_quot(b: u8) -> bool {
	!is_attval_quot_delimiter(b)
}

pub fn is_nonchar_byte(b: u8) -> bool {
	b <= 0x08 || b == 0x0b || b == 0x0c || (b >= 0x0e && b <= 0x1f)
}

/// Valid XML decimal characters (for character references)
pub fn is_decimal_digit(b: u8) -> bool {
	b >= b'0' && b <= b'9'
}

/// Valid XML hexadecimal characters (for character references)
pub fn is_hexadecimal_digit(b: u8) -> bool {
	(b >= b'0' && b <= b'9') || (b >= b'a' && b <= b'f') || (b >= b'A' && b <= b'F')
}

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
static CLASS_XML_CDATA_SECTION_CONTENTS_DELIMITED: CodepointRanges =
	CodepointRanges(VALID_XML_CDATA_RANGES_CDATASECTION_DELIMITED);

#[cfg(test)]
mod tests {
	use super::*;
	use rxml_validation::selectors::*;

	#[test]
	fn test_nonchar_byte_range_is_superset_of_nonchar_codepoint_range() {
		let mut buf = [0u8; 4];
		for cp in 0x0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(cp) {
				let s = ch.encode_utf8(&mut buf[..]);
				if !CLASS_XML_NONCHAR.select(ch) {
					let mut ok = true;
					for b in s.as_bytes() {
						if is_nonchar_byte(*b) {
							ok = false;
						}
					}
					if !ok {
						panic!("byte selector rejects any byte of U+{:04x}", cp);
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
					if class.select(ch) && is_text_delimiter(*b) {
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
					if CLASS_XML_CDATA_SECTION_CONTENTS_DELIMITED.select(ch)
						&& is_cdata_content_delimiter(*b)
					{
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
					if CLASS_XML_NAME.select(ch) && is_name_delimiter(*b) {
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
					if class.select(ch) && is_attval_apos_delimiter(*b) {
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
					if class.select(ch) && is_attval_quot_delimiter(*b) {
						panic!("byte selector rejects byte 0x{:02x}, which is a utf-8 byte of U+{:04x}", *b, cp);
					}
				}
			}
		}
	}
}
