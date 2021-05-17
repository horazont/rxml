/*!
# Strongly-typed strings for use with XML 1.0 documents

This module defines various string- and str-like types which represent pieces
of text as they may occur in XML documents. These types are checked to contain
only text which conforms to the respective grammar in the XML specifications.

This allows to carry information about the checking which already took place
in the parser to the application, avoiding the need to execute checks multiple
times.

## Type Overview

- [`Name`] and [`NameStr`] represent the `Name` production and can be used
  for element and attribute names before namespace prefix expansion.
- [`NCName`] and [`NCNameStr`] represent the `Name` production but without a
  colon inside; they are used for localnames after prefix expansion and to
  carry the prefixes themselves.
- [`CData`] and [`CDataStr`] represent strings of XML `Char`s, which are
  slightly more restrictive than Rust `char`. They are used for attribute
  values and text nodes.

  Note that [`CData`] strings do not contain references or CDATA sections;
  those are expanded by the lexer.
*/

use std::ops::Deref;
use std::borrow::Borrow;
use crate::selectors;
use crate::selectors::{CharSelector, CodepointRanges};
use crate::error::{NWFError, WFError, ERRCTX_UNKNOWN};

static CDATA_ANTI_SELECTOR: CodepointRanges = CodepointRanges(selectors::INVALID_XML_CDATA_RANGES);

fn validate_name(s: &str) -> Result<(), WFError> {
	let mut chars = s.chars();
	match chars.next() {
		// must have at least one char
		None => return Err(WFError::InvalidSyntax("Name must have at least one Char")),
		Some(c) => if !selectors::CLASS_XML_NAMESTART.select(c) {
			return Err(WFError::UnexpectedChar(ERRCTX_UNKNOWN, c, None))
		}
	}
	for ch in chars {
		if !selectors::CLASS_XML_NAME.select(ch) {
			return Err(WFError::UnexpectedChar(ERRCTX_UNKNOWN, ch, None))
		}
	}
	Ok(())
}

fn validate_ncname(s: &str) -> Result<(), WFError> {
	let mut chars = s.chars();
	match chars.next() {
		// must have at least one char
		None => return Err(WFError::InvalidSyntax("Name must have at least one Char")),
		Some(c) => if !selectors::CLASS_XML_NAMESTART.select(c) || c == ':' {
			return Err(WFError::UnexpectedChar(ERRCTX_UNKNOWN, c, None))
		}
	}
	for ch in chars {
		if !selectors::CLASS_XML_NAME.select(ch) || ch == ':' {
			return Err(WFError::UnexpectedChar(ERRCTX_UNKNOWN, ch, None))
		}
	}
	Ok(())
}

fn validate_cdata(s: &str) -> Result<(), WFError> {
	for ch in s.chars() {
		if CDATA_ANTI_SELECTOR.select(ch) {
			return Err(WFError::UnexpectedChar(ERRCTX_UNKNOWN, ch, None))
		}
	}
	Ok(())
}

/// String which conforms to the Name production of XML 1.0.
///
/// [`Name`] corresponds to a (restricted) [`String`]. For a [`str`]-like type
/// with the same restrictions, see [`NameStr`].
///
/// Since [`Name`] derefs to [`String`], all (non-mutable) methods from
/// [`String`] are available.
///
/// # Formal definition
///
/// The data inside [`Name`] (and [`NameStr`]) is guaranteed to conform to
/// the `Name` production of the below grammar, quoted from
/// [XML 1.0 § 2.3](https://www.w3.org/TR/REC-xml/#NT-NameStartChar):
///
/// ```text
/// [4]  NameStartChar ::= ":" | [A-Z] | "_" | [a-z] | [#xC0-#xD6]
///                        | [#xD8-#xF6] | [#xF8-#x2FF] | [#x370-#x37D]
///                        | [#x37F-#x1FFF] | [#x200C-#x200D]
///                        | [#x2070-#x218F] | [#x2C00-#x2FEF]
///                        | [#x3001-#xD7FF] | [#xF900-#xFDCF]
///                        | [#xFDF0-#xFFFD] | [#x10000-#xEFFFF]
/// [4a] NameChar      ::= NameStartChar | "-" | "." | [0-9] | #xB7
///                        | [#x0300-#x036F] | [#x203F-#x2040]
/// [5]  Name          ::= NameStartChar (NameChar)*
/// ```
#[derive(Hash, PartialEq, Debug, Clone)]
pub struct Name(String);

impl Name {
	/// Wrap a given [`String`] in a [`Name`].
	///
	/// This function enforces that the given string conforms to the `Name`
	/// production of XML 1.0. If those conditions are not met, an error is
	/// returned.
	pub fn from_string(s: String) -> Result<Name, WFError> {
		validate_name(s.as_str())?;
		Ok(Name(s))
	}

	/// Copy a given [`str`]-like into a new [`Name`].
	///
	/// This function enforces that the given string conforms to the `Name`
	/// production of XML 1.0. If those conditions are not met, an error is
	/// returned.
	pub fn from_str<T: AsRef<str>>(s: T) -> Result<Name, WFError> {
		let s = s.as_ref();
		validate_name(s)?;
		Ok(Name(s.to_string()))
	}

	/// Split the name at a colon, if it exists.
	///
	/// If the name contains no colon, the function returns `(None, self)`.
	/// If the name contains exactly one colon, the function returns the part
	/// before the colon (the prefix) in the first return value and the part
	/// following the colon (the suffix) as second return value.
	///
	/// If neither of the two cases apply or the string on either side of the
	/// colon is empty, an error is returned.
	pub fn split_name(self) -> Result<(Option<NCName>, NCName), NWFError> {
		let mut name = self.0;
		let colon_pos = match name.find(':') {
			None => return Ok((None, unsafe { NCName::from_string_unchecked(name) })),
			Some(pos) => pos,
		};
		if colon_pos == 0 || colon_pos == name.len() - 1 {
			return Err(NWFError::EmptyNamePart(ERRCTX_UNKNOWN));
		}

		let localname = name.split_off(colon_pos+1);
		let mut prefix = name;

		if localname.find(':').is_some() {
			// Namespaces in XML 1.0 (Third Edition) namespace-well-formed criterium 1
			return Err(NWFError::MultiColonName(ERRCTX_UNKNOWN))
		};
		if !selectors::CLASS_XML_NAMESTART.select(localname.chars().next().unwrap()) {
			// Namespaces in XML 1.0 (Third Edition) NCName production
			return Err(NWFError::InvalidLocalName(ERRCTX_UNKNOWN))
		}

		prefix.pop();
		// do not shrink to fit here -- the prefix will be used when the element
		// is finalized to put it on the stack for quick validation of the
		// </element> token.

		debug_assert!(prefix.len() > 0);
		debug_assert!(localname.len() > 0);
		Ok((
			Some(unsafe { NCName::from_string_unchecked(prefix) }),
			unsafe { NCName::from_string_unchecked(localname) },
		))
	}

	/// Consume the Name and return the internal String
	pub fn as_string(self) -> String {
		self.0
	}

	/// Construct a Name without enforcing anything
	#[doc(hidden)]
	pub unsafe fn from_string_unchecked(s: String) -> Name {
		Name(s)
	}
}

impl Eq for Name {}

impl PartialEq<Name> for &str {
	fn eq(&self, other: &Name) -> bool {
		return self == &other.0.as_str()
	}
}

impl PartialEq<String> for Name {
	fn eq(&self, other: &String) -> bool {
		return self.0 == *other
	}
}

impl PartialEq<&str> for Name {
	fn eq(&self, other: &&str) -> bool {
		return self.0.as_str() == *other
	}
}

impl PartialEq<str> for Name {
	fn eq(&self, other: &str) -> bool {
		return self.0.as_str() == other
	}
}

impl Deref for Name {
	type Target = String;

	fn deref(&self) -> &String {
		&self.0
	}
}

impl AsRef<String> for Name {
	fn as_ref(&self) -> &String {
		&self.0
	}
}

impl AsRef<NameStr> for Name {
	fn as_ref(&self) -> &NameStr {
		unsafe { NameStr::from_str_unchecked(self) }
	}
}

impl Borrow<String> for Name {
	fn borrow(&self) -> &String {
		&self.0
	}
}

impl Borrow<str> for Name {
	fn borrow(&self) -> &str {
		&self.0
	}
}

impl Borrow<NameStr> for Name {
	fn borrow(&self) -> &NameStr {
		unsafe { NameStr::from_str_unchecked(self) }
	}
}

impl From<Name> for String {
	fn from(other: Name) -> String {
		other.0
	}
}

/// str which conforms to the Name production of XML 1.0.
///
/// [`NameStr`] corresponds to a (restricted) [`str`]. For a [`String`]-like
/// type with the same restrictions as well as the formal definition of those
/// restrictions, see [`Name`].
///
/// Since [`NameStr`] derefs to [`str`], all (non-mutable) methods from
/// [`str`] are available.
#[derive(Hash, PartialEq)]
#[repr(transparent)]
pub struct NameStr(str);

impl NameStr {
	/// Wrap a given `str` in a [`NameStr`].
	///
	/// This function enforces that the given string conforms to the `Name`
	/// production of XML 1.0. If those conditions are not met, an error is
	/// returned.
	pub fn from_str<'x>(s: &'x str) -> Result<&'x NameStr, WFError> {
		validate_name(s)?;
		Ok(unsafe { std::mem::transmute(s) })
	}

	/// Copy the NameStr into a new Name.
	pub fn to_name(&self) -> Name {
		unsafe { Name::from_string_unchecked(self.to_string()) }
	}

	/// Construct a NameStr without enforcing anything
	#[doc(hidden)]
	pub unsafe fn from_str_unchecked<'x>(s: &'x str) -> &'x NameStr {
		std::mem::transmute(s)
	}
}

impl Eq for NameStr {}

impl Deref for NameStr {
	type Target = str;

	fn deref(&self) -> &str {
		&self.0
	}
}

impl AsRef<str> for NameStr {
	fn as_ref(&self) -> &str {
		&self.0
	}
}

impl AsRef<[u8]> for NameStr {
	fn as_ref(&self) -> &[u8] {
		self.0.as_bytes()
	}
}

/// String which conforms to the NCName production of Namespaces in XML 1.0.
///
/// [`NCName`] corresponds to a (restricted) [`String`]. For a [`str`]-like
/// type with the same restrictions, see [`NCNameStr`].
///
/// Since [`NCName`] derefs to [`String`], all (non-mutable) methods from
/// [`String`] are available.
///
/// # Formal definition
///
/// The data inside [`NCName`] (and [`NCNameStr`]) is guaranteed to conform to
/// the `NCName` production of the below grammar, quoted from
/// [Namespaces in XML 1.0 § 3](https://www.w3.org/TR/REC-xml-names/#NT-NCName):
///
/// ```text
/// [4] NCName ::= Name - (Char* ':' Char*)  /* An XML Name, minus the ":" */
/// ```
#[derive(Hash, PartialEq, Debug, Clone)]
pub struct NCName(String);

impl NCName {
	/// Wrap a given [`String`] in a [`NCName`].
	///
	/// This function enforces that the given string conforms to the `NCName`
	/// production of Namespaces in XML 1.0. If those conditions are not met,
	/// an error is returned.
	pub fn from_string(s: String) -> Result<NCName, WFError> {
		validate_ncname(s.as_str())?;
		Ok(NCName(s))
	}

	/// Copy a given [`str`]-like into a new [`NCName`].
	///
	/// This function enforces that the given string conforms to the `NCName`
	/// production of Namespaces in XML 1.0. If those conditions are not met,
	/// an error is returned.
	pub fn from_str<T: AsRef<str>>(s: T) -> Result<NCName, WFError> {
		let s = s.as_ref();
		validate_ncname(s)?;
		Ok(NCName(s.to_string()))
	}

	/// Compose two [`NCName`] objects to one [`Name`], separating them with
	/// a colon.
	///
	/// As an [`NCName`] is always a valid [`Name`], the composition of the
	/// two with a `:` as separator is also a valid [`Name`].
	///
	/// This is the inverse of [`Name::split_name()`].
	///
	/// # Example
	///
	/// ```
	/// # use rxml::NCName;
	/// let prefix = NCName::from_str("xmlns").unwrap();
	/// let localname = NCName::from_str("stream").unwrap();
	/// assert_eq!(prefix.add_suffix(&localname), "xmlns:stream");
	/// ```
	pub fn add_suffix(self, suffix: &NCName) -> Name {
		let mut s = self.0;
		s.reserve(suffix.len() + 1);
		s.push_str(":");
		s.push_str(suffix.as_str());
		unsafe { Name::from_string_unchecked(s) }
	}

	pub fn as_name(self) -> Name {
		unsafe { Name::from_string_unchecked(self.0) }
	}

	/// Consume the NCName and return the internal String
	pub fn as_string(self) -> String {
		self.0
	}

	/// Construct an NCName without enforcing anything
	#[doc(hidden)]
	pub unsafe fn from_string_unchecked(s: String) -> NCName {
		NCName(s)
	}
}

impl Eq for NCName {}

impl PartialEq<NCName> for &str {
	fn eq(&self, other: &NCName) -> bool {
		return self == &other.0.as_str()
	}
}

impl PartialEq<String> for NCName {
	fn eq(&self, other: &String) -> bool {
		return self.0 == *other
	}
}

impl PartialEq<&str> for NCName {
	fn eq(&self, other: &&str) -> bool {
		return self.0.as_str() == *other
	}
}

impl PartialEq<str> for NCName {
	fn eq(&self, other: &str) -> bool {
		return self.0.as_str() == other
	}
}

impl Deref for NCName {
	type Target = String;

	fn deref(&self) -> &String {
		&self.0
	}
}

impl AsRef<String> for NCName {
	fn as_ref(&self) -> &String {
		&self.0
	}
}

impl AsRef<NCNameStr> for NCName {
	fn as_ref(&self) -> &NCNameStr {
		unsafe { NCNameStr::from_str_unchecked(&self.0) }
	}
}

impl Borrow<String> for NCName {
	fn borrow(&self) -> &String {
		&self.0
	}
}

impl Borrow<str> for NCName {
	fn borrow(&self) -> &str {
		&self.0
	}
}

impl Borrow<NCNameStr> for NCName {
	fn borrow(&self) -> &NCNameStr {
		unsafe { NCNameStr::from_str_unchecked(&self.0) }
	}
}

impl From<NCName> for Name {
	fn from(other: NCName) -> Name {
		Name(other.0)
	}
}

impl From<NCName> for String {
	fn from(other: NCName) -> String {
		other.0
	}
}

/// str which conforms to the NCName production of Namespaces in XML 1.0.
///
/// [`NCNameStr`] corresponds to a (restricted) [`str`]. For a [`String`]-like
/// type with the same restrictions as well as the formal definition of those
/// restrictions, see [`NCName`].
///
/// Since [`NCNameStr`] derefs to [`str`], all (non-mutable) methods from
/// [`str`] are available.
#[derive(Hash, PartialEq)]
#[repr(transparent)]
pub struct NCNameStr(str);

impl NCNameStr {
	/// Wrap a str in a NCNameStr
	///
	/// This function enforces that the given string conforms to the `NCName`
	/// production of Namespaces in XML 1.0. If those conditions are not met,
	/// an error is returned.
	pub fn from_str<'x>(s: &'x str) -> Result<&'x NCNameStr, WFError> {
		validate_name(s)?;
		Ok(unsafe { std::mem::transmute(s) })
	}

	/// Copy the NCNameStr into a new NCName
	pub fn to_ncname(&self) -> NCName {
		unsafe { NCName::from_string_unchecked(self.0.to_string()) }
	}

	/// Construct a NCNameStr without checking anything
	#[doc(hidden)]
	pub unsafe fn from_str_unchecked<'x>(s: &'x str) -> &'x NCNameStr {
		std::mem::transmute(s)
	}
}

impl Eq for NCNameStr {}

impl Deref for NCNameStr {
	type Target = str;

	fn deref(&self) -> &str {
		&self.0
	}
}

impl AsRef<str> for NCNameStr {
	fn as_ref(&self) -> &str {
		&self.0
	}
}

impl AsRef<[u8]> for NCNameStr {
	fn as_ref(&self) -> &[u8] {
		self.0.as_bytes()
	}
}

/// String which consists only of XML 1.0 Chars.
///
/// [`CData`] corresponds to a (restricted) [`String`]. For a [`str`]-like
/// type with the same restrictions, see [`CDataStr`].
///
/// Since [`CData`] derefs to [`String`], all (non-mutable) methods from
/// [`String`] are available.
///
/// # Formal definition
///
/// The data inside [`CData`] (and [`CDataStr`]) is guaranteed to be a
/// sequence of `Char` as defined in
/// [XML 1.0 § 2.2](https://www.w3.org/TR/REC-xml/#NT-Char) and quoted below:
///
/// ```text
/// [2] Char ::= #x9 | #xA | #xD | [#x20-#xD7FF] | [#xE000-#xFFFD]
///              | [#x10000-#x10FFFF]
///              /* any Unicode character, excluding the surrogate blocks,
///                 FFFE, and FFFF. */
/// ```
///
/// This is a Unicode scalar value, minus ASCII control characters except
/// Tab (`\x09`), CR (`\x0d`) and LF (`\x0a`), the BOM (`\u{fffe}`) and
/// whatever `\u{ffff}` is.
#[derive(Hash, PartialEq, Debug, Clone)]
pub struct CData(String);

impl CData {
	/// Wrap a given [`String`] in a [`CData`].
	///
	/// This function enforces that the chars in the string conform to `Char`
	/// as defined in XML 1.0. If those conditions are not met, an error is
	/// returned.
	pub fn from_string(s: String) -> Result<CData, WFError> {
		validate_cdata(s.as_str())?;
		Ok(CData(s))
	}

	/// Copy a given [`str`]-like into a new [`NCName`].
	///
	/// This function enforces that the chars in the string conform to `Char`
	/// as defined in XML 1.0. If those conditions are not met, an error is
	/// returned.
	pub fn from_str<T: AsRef<str>>(s: T) -> Result<CData, WFError> {
		let s = s.as_ref();
		validate_cdata(s)?;
		Ok(CData(s.to_string()))
	}

	pub fn as_cdata_str(&self) -> &CDataStr {
		unsafe { CDataStr::from_str_unchecked(&self.0) }
	}

	/// Consume the CData and return the internal String
	pub fn as_string(self) -> String {
		self.0
	}

	/// Construct a CData without checking anything.
	#[doc(hidden)]
	pub unsafe fn from_string_unchecked(s: String) -> CData {
		CData(s)
	}
}

impl Eq for CData {}

impl PartialEq<CData> for &str {
	fn eq(&self, other: &CData) -> bool {
		return self == &other.0.as_str()
	}
}

impl PartialEq<String> for CData {
	fn eq(&self, other: &String) -> bool {
		return self.0 == *other
	}
}

impl PartialEq<&str> for CData {
	fn eq(&self, other: &&str) -> bool {
		return self.0.as_str() == *other
	}
}

impl PartialEq<str> for CData {
	fn eq(&self, other: &str) -> bool {
		return self.0.as_str() == other
	}
}

impl PartialEq<&CDataStr> for CData {
	fn eq(&self, other: &&CDataStr) -> bool {
		return self.0.as_str() == &other.0
	}
}

impl PartialEq<CDataStr> for CData {
	fn eq(&self, other: &CDataStr) -> bool {
		return self.0.as_str() == &other.0
	}
}

impl Deref for CData {
	type Target = String;

	fn deref(&self) -> &String {
		&self.0
	}
}

impl AsRef<String> for CData {
	fn as_ref(&self) -> &String {
		&self.0
	}
}

impl AsRef<CDataStr> for CData {
	fn as_ref(&self) -> &CDataStr {
		self.as_cdata_str()
	}
}

impl Borrow<String> for CData {
	fn borrow(&self) -> &String {
		&self.0
	}
}

impl Borrow<str> for CData {
	fn borrow(&self) -> &str {
		&self.0
	}
}

impl Borrow<CDataStr> for CData {
	fn borrow(&self) -> &CDataStr {
		self.as_cdata_str()
	}
}

impl From<CData> for Name {
	fn from(other: CData) -> Name {
		Name(other.0)
	}
}

impl From<CData> for String {
	fn from(other: CData) -> String {
		other.0
	}
}

/// str which consists only of XML 1.0 Chars.
///
/// [`CDataStr`] corresponds to a (restricted) [`str`]. For a [`String`]-like
/// type with the same restrictions as well as the formal definition of those
/// restrictions, see [`CData`].
///
/// Since [`CDataStr`] derefs to [`str`], all (non-mutable) methods from
/// [`str`] are available.
#[derive(Hash, PartialEq)]
#[repr(transparent)]
pub struct CDataStr(str);

impl CDataStr {
	/// Wrap a str in a CDataStr
	///
	/// This function enforces that the chars in the string conform to `Char`
	/// as defined in XML 1.0. If those conditions are not met, an error is
	/// returned.
	pub fn from_str<'x>(s: &'x str) -> Result<&'x CDataStr, WFError> {
		validate_name(s)?;
		Ok(unsafe { std::mem::transmute(s) })
	}

	/// Copy the CDataStr into a new CData
	pub fn to_cdata(&self) -> CData {
		unsafe { CData::from_string_unchecked(self.0.to_string()) }
	}

	#[doc(hidden)]
	pub unsafe fn from_str_unchecked<'x>(s: &'x str) -> &'x CDataStr {
		std::mem::transmute(s)
	}
}

impl Eq for CDataStr {}

impl Deref for CDataStr {
	type Target = str;

	fn deref(&self) -> &str {
		&self.0
	}
}

impl AsRef<str> for CDataStr {
	fn as_ref(&self) -> &str {
		&self.0
	}
}

impl AsRef<[u8]> for CDataStr {
	fn as_ref(&self) -> &[u8] {
		self.0.as_bytes()
	}
}

impl PartialEq<str> for CDataStr {
	fn eq(&self, other: &str) -> bool {
		&self.0 == other
	}
}

impl PartialEq<CDataStr> for str {
	fn eq(&self, other: &CDataStr) -> bool {
		self == &other.0
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn split_name_rejects_localname_with_non_namestart_first_char() {
		let nm = Name::from_str("foo:-bar").unwrap();
		let result = nm.split_name();
		assert!(matches!(result.err().unwrap(), NWFError::InvalidLocalName(_)));
	}
}
