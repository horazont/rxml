use rxml_validation::selectors::{CodepointRange, CodepointRanges};

/// Valid codepoints for XML character data minus delimiters (XML 1.0 § 2.4 [14])
///
/// This is like [`VALID_XML_CDATA_RANGES`], but the following chars are excluded:
///
/// - `'\r'`, because that gets folded into a line feed (`\n`) on input
/// - `'&'`, because that may start an entity or character reference
/// - `'<'`, because that may start an element or CDATA section
/// - `']'`, because that may end a CDATA section and the sequence `]]>` is not allowed verbatimly in character data in XML documents
pub const VALID_XML_CDATA_RANGES_TEXT_DELIMITED: &'static [CodepointRange] = &[
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
pub const VALID_XML_CDATA_RANGES_ATT_APOS_DELIMITED: &'static [CodepointRange] = &[
	// exclude all whitespace except normal space because those get converted into spaces
	CodepointRange('\u{0020}', '\u{0025}'), // excludes &, '
	CodepointRange('\u{0028}', '\u{003b}'), // excludes <
	CodepointRange('\u{003d}', '\u{d7ff}'),
	CodepointRange('\u{e000}', '\u{fffd}'),
	CodepointRange('\u{10000}', '\u{10ffff}'),
];


// XML 1.0 § 2.3 [10]
pub const VALID_XML_CDATA_RANGES_ATT_QUOT_DELIMITED: &'static [CodepointRange] = &[
	// exclude all whitespace except normal space because those get converted into spaces
	CodepointRange('\u{0020}', '\u{0021}'), // excludes "
	CodepointRange('\u{0023}', '\u{0025}'), // excludes &
	CodepointRange('\u{0027}', '\u{003b}'), // excludes <
	CodepointRange('\u{003d}', '\u{d7ff}'),
	CodepointRange('\u{e000}', '\u{fffd}'),
	CodepointRange('\u{10000}', '\u{10ffff}'),
];


// XML 1.0 § 2.4 [14]
pub const VALID_XML_CDATA_RANGES_CDATASECTION_DELIMITED: &'static [CodepointRange] = &[
	CodepointRange('\x09', '\x0a'),
	// excluding CR as that gets folded to LF
	CodepointRange('\u{0020}', '\u{005c}'), // excludes ]
	CodepointRange('\u{005e}', '\u{d7ff}'),
	CodepointRange('\u{e000}', '\u{fffd}'),
	CodepointRange('\u{10000}', '\u{10ffff}'),
];

const VALID_XML_HEXADECIMALS: &'static [CodepointRange] = &[
	CodepointRange('A', 'F'),
	CodepointRange('0', '9'),
	CodepointRange('a', 'f'),
];

pub static CLASS_XML_CDATA_SECTION_CONTENTS_DELIMITED: CodepointRanges = CodepointRanges(VALID_XML_CDATA_RANGES_CDATASECTION_DELIMITED);

/// XML whitespace
pub static CLASS_XML_SPACES: &'static [char] = &[' ', '\t', '\r', '\n'];

/// Valid XML decimal characters (for character references)
pub const CLASS_XML_DECIMAL_DIGITS: CodepointRange = CodepointRange('0', '9');

/// Valid XML hexadecimal characters (for character references)
pub static CLASS_XML_HEXADECIMAL_DIGITS: CodepointRanges = CodepointRanges(VALID_XML_HEXADECIMALS);
