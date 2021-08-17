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

## Construction

To construct [`str`]-like references at compile time, you can use the macros
from the [`rxml_proc`](https://docs.rs/rxml_proc) crate. It offers
`xml_cdata!`, `xml_name!` and `xml_ncname!` macros which convert a string
literal into `&CDataStr`, `&NameStr` and `&NCNameStr` (respectively) with
validation at compile time.

In general, owned values are constructed using the [`std::convert::TryInto`]
mechanism, from other string types. Supported source types are:

* [`String`] (copies for [`Name`] and [`NCName`], moves for [`CData`])
* [`smartstring::alias::String`] (copies for [`CData`], moves for [`Name`] and [`NCName])
* [`str`] (copies for all types except the slice types)

In addition, the following conversions can be done without extra checking and
are possible through `.into()`:

* [`NCName`] to [`Name`]
* [`NCName`] to [`CData`]
* [`Name`] to [`CData`]

(and likewise for the corresponding Str types)

The inverse directions are only available through `try_into`.
*/

use std::ops::Deref;
use std::fmt;
use std::convert::{TryFrom, TryInto};
use std::borrow::{Borrow, ToOwned, Cow};
use crate::error::{NWFError, WFError, ERRCTX_UNKNOWN};
use smartstring::alias::String as SmartString;
use rxml_validation::selectors;
use rxml_validation::selectors::CharSelector;
pub use rxml_validation::{validate_name, validate_ncname, validate_cdata};

/// String which conforms to the Name production of XML 1.0.
///
/// [`Name`] corresponds to a (restricted) [`String`]. For a [`str`]-like type
/// with the same restrictions, see [`NameStr`]. `&NameStr` can be created
/// from a string literal at compile time using the `xml_name` macro from
/// [`rxml_proc`](https://docs.rs/rxml_proc).
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
pub struct Name(SmartString);

impl Name {
	/// Wrap a given [`String`] in a [`Name`].
	///
	/// This function enforces that the given string conforms to the `Name`
	/// production of XML 1.0. If those conditions are not met, an error is
	/// returned.
	#[deprecated(since = "0.4.0", note = "use the TryFrom<> trait implementation instead")]
	pub fn from_string<T: Into<String>>(s: T) -> Result<Name, WFError> {
		s.into().try_into()
	}

	/// Wrap a given [`smartstring::SmartString`] in a [`Name`].
	///
	/// This function enforces that the given string conforms to the `Name`
	/// production of XML 1.0. If those conditions are not met, an error is
	/// returned.
	#[deprecated(since = "0.4.0", note = "use the TryFrom<> trait implementation instead")]
	pub fn from_smartstring<T: Into<SmartString>>(s: T) -> Result<Name, WFError> {
		s.into().try_into()
	}

	/// Copy a given [`str`]-like into a new [`Name`].
	///
	/// This function enforces that the given string conforms to the `Name`
	/// production of XML 1.0. If those conditions are not met, an error is
	/// returned.
	#[deprecated(since = "0.4.0", note = "use the TryFrom<> trait implementation instead")]
	pub fn from_str<T: AsRef<str>>(s: T) -> Result<Name, WFError> {
		s.as_ref().try_into()
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
			None => return Ok((None, unsafe { NCName::from_smartstring_unchecked(name) })),
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
			Some(unsafe { NCName::from_smartstring_unchecked(prefix) }),
			unsafe { NCName::from_smartstring_unchecked(localname) },
		))
	}

	/// Consume the Name and return the internal String
	pub fn as_string(self) -> String {
		self.0.into()
	}

	/// Construct a Name without enforcing anything
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// a valid Name.
	pub unsafe fn from_str_unchecked<T: AsRef<str>>(s: T) -> Name {
		Name(s.as_ref().into())
	}

	/// Construct a Name without enforcing anything
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// a valid Name.
	pub unsafe fn from_string_unchecked<T: Into<String>>(s: T) -> Name {
		Name(s.into().into())
	}

	/// Construct a Name without enforcing anything
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// a valid Name.
	pub unsafe fn from_smartstring_unchecked<T: Into<SmartString>>(s: T) -> Name {
		Name(s.into())
	}
}

impl Eq for Name {}

impl PartialEq<Name> for &str {
	fn eq(&self, other: &Name) -> bool {
		return self == &other.0
	}
}

impl PartialEq<&str> for Name {
	fn eq(&self, other: &&str) -> bool {
		return &self.0 == *other
	}
}

impl PartialEq<Name> for str {
	fn eq(&self, other: &Name) -> bool {
		return self == other.0
	}
}

impl PartialEq<str> for Name {
	fn eq(&self, other: &str) -> bool {
		return self.0 == other
	}
}

impl PartialEq<Name> for &NameStr {
	fn eq(&self, other: &Name) -> bool {
		return self.0 == other.0
	}
}

impl PartialEq<&NameStr> for Name {
	fn eq(&self, other: &&NameStr) -> bool {
		return &self.0 == &other.0
	}
}

impl PartialEq<Name> for NameStr {
	fn eq(&self, other: &Name) -> bool {
		return self.0 == other.0
	}
}

impl PartialEq<NameStr> for Name {
	fn eq(&self, other: &NameStr) -> bool {
		return self.0 == other.0
	}
}

impl Deref for Name {
	type Target = SmartString;

	fn deref(&self) -> &SmartString {
		&self.0
	}
}

impl AsRef<SmartString> for Name {
	fn as_ref(&self) -> &SmartString {
		&self.0
	}
}

impl AsRef<NameStr> for Name {
	fn as_ref(&self) -> &NameStr {
		unsafe { NameStr::from_str_unchecked(self) }
	}
}

impl AsRef<str> for Name {
	fn as_ref(&self) -> &str {
		&self.0
	}
}

impl Borrow<SmartString> for Name {
	fn borrow(&self) -> &SmartString {
		&self.0
	}
}

impl Borrow<NameStr> for Name {
	fn borrow(&self) -> &NameStr {
		unsafe { NameStr::from_str_unchecked(self) }
	}
}

impl Borrow<str> for Name {
	fn borrow(&self) -> &str {
		&self.0
	}
}

impl From<Name> for String {
	fn from(other: Name) -> String {
		other.0.into()
	}
}

impl From<Name> for SmartString {
	fn from(other: Name) -> SmartString {
		other.0
	}
}

impl<'x> From<Name> for Cow<'x, NameStr> {
	fn from(other: Name) -> Cow<'x, NameStr> {
		Cow::Owned(other)
	}
}

impl<'x> From<Cow<'x, NameStr>> for Name {
	fn from(other: Cow<'x, NameStr>) -> Name {
		other.into_owned()
	}
}

impl From<NCName> for Name {
	fn from(other: NCName) -> Name {
		Name(other.0)
	}
}

impl TryFrom<SmartString> for Name  {
	type Error = WFError;

	fn try_from(other: SmartString) -> Result<Self, Self::Error> {
		validate_name(&other)?;
		Ok(Name(other))
	}
}

impl TryFrom<String> for Name  {
	type Error = WFError;

	fn try_from(other: String) -> Result<Self, Self::Error> {
		validate_name(&other)?;
		Ok(Name(other.into()))
	}
}

impl TryFrom<&str> for Name  {
	type Error = WFError;

	fn try_from(other: &str) -> Result<Self, Self::Error> {
		validate_name(other)?;
		Ok(Name(other.into()))
	}
}

impl fmt::Display for Name {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		f.write_str(&self.0 as &str)
	}
}

/// str which conforms to the Name production of XML 1.0.
///
/// [`NameStr`] corresponds to a (restricted) [`str`]. For a [`String`]-like
/// type with the same restrictions as well as the formal definition of those
/// restrictions, see [`Name`].
///
/// `&NameStr` can be created from a string literal at compile time using the
/// `xml_name` macro from [`rxml_proc`](https://docs.rs/rxml_proc).
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
	#[deprecated(since = "0.4.0", note = "use the TryFrom<> trait implementation or xml_name! macro instead")]
	pub fn from_str<'x>(s: &'x str) -> Result<&'x NameStr, WFError> {
		s.try_into()
	}

	/// Copy the NameStr into a new Name.
	pub fn to_name(&self) -> Name {
		unsafe { Name::from_string_unchecked(self.to_string()) }
	}

	/// Construct a NameStr without enforcing anything
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// a valid Name.
	pub unsafe fn from_str_unchecked<'x>(s: &'x str) -> &'x NameStr {
		std::mem::transmute(s)
	}
}

impl Eq for NameStr {}

impl PartialEq<NameStr> for &str {
	fn eq(&self, other: &NameStr) -> bool {
		return *self == &other.0
	}
}

impl PartialEq<&str> for NameStr {
	fn eq(&self, other: &&str) -> bool {
		return &self.0 == *other
	}
}

impl PartialEq<NameStr> for str {
	fn eq(&self, other: &NameStr) -> bool {
		return self == &other.0
	}
}

impl PartialEq<str> for NameStr {
	fn eq(&self, other: &str) -> bool {
		return &self.0 == other
	}
}

impl Deref for NameStr {
	type Target = str;

	fn deref(&self) -> &str {
		&self.0
	}
}

impl AsRef<NameStr> for NameStr {
	fn as_ref(&self) -> &Self {
		&self
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

impl ToOwned for NameStr {
	type Owned = Name;

	fn to_owned(&self) -> Self::Owned {
		self.into()
	}
}

impl From<&NameStr> for String {
	fn from(other: &NameStr) -> String {
		other.0.to_string()
	}
}

impl From<&NameStr> for SmartString {
	fn from(other: &NameStr) -> SmartString {
		other.0.into()
	}
}

impl From<&NameStr> for Name {
	fn from(other: &NameStr) -> Name {
		unsafe { Name::from_str_unchecked(&other.0) }
	}
}

impl<'x> TryFrom<&'x str> for &'x NameStr {
	type Error = WFError;

	fn try_from(other: &'x str) -> Result<Self, Self::Error> {
		validate_name(other)?;
		Ok(unsafe { std::mem::transmute(other) })
	}
}

impl fmt::Display for NameStr {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		f.write_str(&self.0)
	}
}

/// String which conforms to the NCName production of Namespaces in XML 1.0.
///
/// [`NCName`] corresponds to a (restricted) [`String`]. For a [`str`]-like
/// type with the same restrictions, see [`NCNameStr`]. `&NCNameStr` can be
/// created from a string literal at compile time using the `xml_ncname` macro
/// from [`rxml_proc`](https://docs.rs/rxml_proc).
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
pub struct NCName(SmartString);

impl NCName {
	/// Wrap a given [`String`] in a [`NCName`].
	///
	/// This function enforces that the given string conforms to the `NCName`
	/// production of Namespaces in XML 1.0. If those conditions are not met,
	/// an error is returned.
	pub fn from_string<T: Into<String>>(s: T) -> Result<NCName, WFError> {
		let s = s.into();
		validate_ncname(&s)?;
		Ok(NCName(s.into()))
	}

	/// Wrap a given [`smartstring::SmartString`] in a [`NCName`].
	///
	/// This function enforces that the given string conforms to the `NCName`
	/// production of Namespaces in XML 1.0. If those conditions are not met,
	/// an error is returned.
	pub fn from_smartstring<T: Into<SmartString>>(s: T) -> Result<NCName, WFError> {
		let s = s.into();
		validate_ncname(&s)?;
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
		Ok(NCName(s.into()))
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
		let mut s: String = self.0.into();
		s.reserve(suffix.len() + 1);
		s.push_str(":");
		s.push_str(suffix.as_str());
		unsafe { Name::from_string_unchecked(s) }
	}

	pub fn as_name(self) -> Name {
		unsafe { Name::from_smartstring_unchecked(self.0) }
	}

	/// Consume the NCName and return the internal String
	pub fn as_string(self) -> String {
		self.0.into()
	}

	/// Construct an NCName without enforcing anything
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// a valid NCName.
	pub unsafe fn from_str_unchecked<T: AsRef<str>>(s: T) -> NCName {
		NCName(s.as_ref().into())
	}

	/// Construct an NCName without enforcing anything
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// a valid NCName.
	pub unsafe fn from_string_unchecked<T: Into<String>>(s: T) -> NCName {
		NCName(s.into().into())
	}

	/// Construct an NCName without enforcing anything
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// a valid NCName.
	pub unsafe fn from_smartstring_unchecked<T: Into<SmartString>>(s: T) -> NCName {
		NCName(s.into())
	}
}

impl Eq for NCName {}

impl PartialEq<NCName> for &str {
	fn eq(&self, other: &NCName) -> bool {
		return self == &other.0
	}
}

impl PartialEq<&str> for NCName {
	fn eq(&self, other: &&str) -> bool {
		return self.0 == *other
	}
}

impl PartialEq<NCName> for str {
	fn eq(&self, other: &NCName) -> bool {
		return self == &other.0
	}
}

impl PartialEq<str> for NCName {
	fn eq(&self, other: &str) -> bool {
		return self.0 == *other
	}
}

impl PartialEq<NCName> for &NCNameStr {
	fn eq(&self, other: &NCName) -> bool {
		return self.0 == other.0
	}
}

impl PartialEq<&NCNameStr> for NCName {
	fn eq(&self, other: &&NCNameStr) -> bool {
		return self.0 == other.0
	}
}

impl PartialEq<NCName> for NCNameStr {
	fn eq(&self, other: &NCName) -> bool {
		return self.0 == other.0
	}
}

impl PartialEq<NCNameStr> for NCName {
	fn eq(&self, other: &NCNameStr) -> bool {
		return self.0 == other.0
	}
}

impl Deref for NCName {
	type Target = SmartString;

	fn deref(&self) -> &SmartString {
		&self.0
	}
}

impl AsRef<SmartString> for NCName {
	fn as_ref(&self) -> &SmartString {
		&self.0
	}
}

impl AsRef<NCNameStr> for NCName {
	fn as_ref(&self) -> &NCNameStr {
		unsafe { NCNameStr::from_str_unchecked(&self.0) }
	}
}

impl AsRef<str> for NCName {
	fn as_ref(&self) -> &str {
		&self.0
	}
}

impl Borrow<SmartString> for NCName {
	fn borrow(&self) -> &SmartString {
		&self.0
	}
}

impl Borrow<NCNameStr> for NCName {
	fn borrow(&self) -> &NCNameStr {
		unsafe { NCNameStr::from_str_unchecked(&self.0) }
	}
}

impl Borrow<str> for NCName {
	fn borrow(&self) -> &str {
		&self.0
	}
}

impl From<NCName> for String {
	fn from(other: NCName) -> String {
		other.0.into()
	}
}

impl From<NCName> for SmartString {
	fn from(other: NCName) -> SmartString {
		other.0
	}
}

impl<'x> From<NCName> for Cow<'x, NCNameStr> {
	fn from(other: NCName) -> Cow<'x, NCNameStr> {
		Cow::Owned(other)
	}
}

impl<'x> From<Cow<'x, NCNameStr>> for NCName {
	fn from(other: Cow<'x, NCNameStr>) -> NCName {
		other.into_owned()
	}
}

impl TryFrom<SmartString> for NCName {
	type Error = WFError;

	fn try_from(other: SmartString) -> Result<Self, Self::Error> {
		NCName::from_smartstring(other)
	}
}

impl TryFrom<String> for NCName {
	type Error = WFError;

	fn try_from(other: String) -> Result<Self, Self::Error> {
		NCName::from_string(other)
	}
}

impl TryFrom<&str> for NCName  {
	type Error = WFError;

	fn try_from(other: &str) -> Result<Self, Self::Error> {
		NCName::from_str(other)
	}
}

impl fmt::Display for NCName {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		f.write_str(&self.0 as &str)
	}
}

/// str which conforms to the NCName production of Namespaces in XML 1.0.
///
/// [`NCNameStr`] corresponds to a (restricted) [`str`]. For a [`String`]-like
/// type with the same restrictions as well as the formal definition of those
/// restrictions, see [`NCName`].
///
/// `&NCNameStr` can be created from a string literal at compile time using
/// the `xml_ncname` macro from [`rxml_proc`](https://docs.rs/rxml_proc).
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
	#[deprecated(since = "0.4.0", note = "use the TryFrom<> trait implementation or xml_ncname! macro instead")]
	pub fn from_str<'x>(s: &'x str) -> Result<&'x NCNameStr, WFError> {
		validate_name(s)?;
		Ok(unsafe { std::mem::transmute(s) })
	}

	/// Copy the NCNameStr into a new NCName
	pub fn to_ncname(&self) -> NCName {
		unsafe { NCName::from_string_unchecked(self.0.to_string()) }
	}

	/// Construct a NCNameStr without checking anything
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// a valid NCName.
	pub unsafe fn from_str_unchecked<'x>(s: &'x str) -> &'x NCNameStr {
		std::mem::transmute(s)
	}
}

impl Eq for NCNameStr {}

impl PartialEq<NCNameStr> for &str {
	fn eq(&self, other: &NCNameStr) -> bool {
		return *self == &other.0
	}
}

impl PartialEq<&str> for NCNameStr {
	fn eq(&self, other: &&str) -> bool {
		return &self.0 == *other
	}
}

impl PartialEq<NCNameStr> for str {
	fn eq(&self, other: &NCNameStr) -> bool {
		return self == &other.0
	}
}

impl PartialEq<str> for NCNameStr {
	fn eq(&self, other: &str) -> bool {
		return &self.0 == other
	}
}

impl Deref for NCNameStr {
	type Target = str;

	fn deref(&self) -> &str {
		&self.0
	}
}

impl AsRef<NCNameStr> for NCNameStr {
	fn as_ref(&self) -> &Self {
		&self
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

impl ToOwned for NCNameStr {
	type Owned = NCName;

	fn to_owned(&self) -> Self::Owned {
		unsafe { NCName::from_str_unchecked(&self.0) }
	}
}

impl From<&NCNameStr> for String {
	fn from(other: &NCNameStr) -> String {
		other.0.into()
	}
}

impl From<&NCNameStr> for SmartString {
	fn from(other: &NCNameStr) -> SmartString {
		other.0.into()
	}
}

impl From<&NCNameStr> for NCName {
	fn from(other: &NCNameStr) -> NCName {
		unsafe { NCName::from_str_unchecked(&other.0) }
	}
}

impl fmt::Display for NCNameStr {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		f.write_str(&self.0)
	}
}

/// String which consists only of XML 1.0 Chars.
///
/// [`CData`] corresponds to a (restricted) [`String`]. For a [`str`]-like
/// type with the same restrictions, see [`CDataStr`]. `&CDataStr` can be
/// created from a string literal at compile time using the `xml_cdata` macro
/// from [`rxml_proc`](https://docs.rs/rxml_proc).
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
///
/// # Escaping
///
/// [`CData`] objects do not contain references or CDATA sections as those are
/// expanded by the lexer. This implies that `CData` objects are not safe to
/// just verbatimly copy into an XML document; additional escaping may be
/// necessary.
#[derive(Hash, PartialEq, Debug, Clone)]
pub struct CData(String);

impl CData {
	/// Wrap a given [`String`] in a [`CData`].
	///
	/// This function enforces that the chars in the string conform to `Char`
	/// as defined in XML 1.0. If those conditions are not met, an error is
	/// returned.
	#[deprecated(since = "0.4.0", note = "use the TryFrom<> trait implementation instead")]
	pub fn from_string<T: Into<String>>(s: T) -> Result<CData, WFError> {
		s.into().try_into()
	}

	/// Wrap a given [`smartstring::SmartString`] in a [`CData`].
	///
	/// This function enforces that the chars in the string conform to `Char`
	/// as defined in XML 1.0. If those conditions are not met, an error is
	/// returned.
	#[deprecated(since = "0.4.0", note = "use the TryFrom<> trait implementation instead")]
	pub fn from_smartstring<T: Into<SmartString>>(s: T) -> Result<CData, WFError> {
		s.into().try_into()
	}

	/// Copy a given [`str`]-like into a new [`NCName`].
	///
	/// This function enforces that the chars in the string conform to `Char`
	/// as defined in XML 1.0. If those conditions are not met, an error is
	/// returned.
	#[deprecated(since = "0.4.0", note = "use the TryFrom<> trait implementation instead")]
	pub fn from_str<T: AsRef<str>>(s: T) -> Result<CData, WFError> {
		s.as_ref().try_into()
	}

	pub fn as_cdata_str(&self) -> &CDataStr {
		unsafe { CDataStr::from_str_unchecked(&self.0) }
	}

	/// Consume the CData and return the internal String
	pub fn as_string(self) -> String {
		self.0
	}

	/// Construct a CData without checking anything.
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// valid CData.
	pub unsafe fn from_str_unchecked<T: AsRef<str>>(s: T) -> CData {
		CData(s.as_ref().into())
	}

	/// Construct a CData without checking anything.
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// valid CData.
	pub unsafe fn from_smartstring_unchecked<T: Into<SmartString>>(s: T) -> CData {
		CData(s.into().into())
	}

	/// Construct a CData without checking anything.
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// valid CData.
	pub unsafe fn from_string_unchecked<T: Into<String>>(s: T) -> CData {
		CData(s.into())
	}
}

impl Eq for CData {}

impl PartialEq<CData> for &str {
	fn eq(&self, other: &CData) -> bool {
		return self == &other.0
	}
}

impl PartialEq<&str> for CData {
	fn eq(&self, other: &&str) -> bool {
		return self.0 == *other
	}
}

impl PartialEq<CData> for str {
	fn eq(&self, other: &CData) -> bool {
		return self == &other.0
	}
}

impl PartialEq<str> for CData {
	fn eq(&self, other: &str) -> bool {
		return self.0 == other
	}
}

impl PartialEq<CData> for &CDataStr {
	fn eq(&self, other: &CData) -> bool {
		return &self.0 == &other.0
	}
}

impl PartialEq<&CDataStr> for CData {
	fn eq(&self, other: &&CDataStr) -> bool {
		return self.0 == &other.0
	}
}

impl PartialEq<CData> for CDataStr {
	fn eq(&self, other: &CData) -> bool {
		return &self.0 == &other.0
	}
}

impl PartialEq<CDataStr> for CData {
	fn eq(&self, other: &CDataStr) -> bool {
		return self.0 == other.0
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

impl AsRef<str> for CData {
	fn as_ref(&self) -> &str {
		&self.0
	}
}

impl Borrow<String> for CData {
	fn borrow(&self) -> &String {
		&self.0
	}
}

impl Borrow<CDataStr> for CData {
	fn borrow(&self) -> &CDataStr {
		self.as_cdata_str()
	}
}

impl Borrow<str> for CData {
	fn borrow(&self) -> &str {
		&self.0
	}
}

impl From<CData> for String {
	fn from(other: CData) -> String {
		other.0
	}
}

impl From<CData> for SmartString {
	fn from(other: CData) -> SmartString {
		other.0.into()
	}
}

impl<'x> From<CData> for Cow<'x, CDataStr> {
	fn from(other: CData) -> Cow<'x, CDataStr> {
		Cow::Owned(other)
	}
}

impl<'x> From<Cow<'x, CDataStr>> for CData {
	fn from(other: Cow<'x, CDataStr>) -> CData {
		other.into_owned()
	}
}

impl From<CData> for Name {
	fn from(other: CData) -> Name {
		Name(other.0.into())
	}
}

impl TryFrom<SmartString> for CData {
	type Error = WFError;

	fn try_from(other: SmartString) -> Result<Self, Self::Error> {
		validate_cdata(&other)?;
		Ok(CData(other.into()))
	}
}

impl TryFrom<String> for CData {
	type Error = WFError;

	fn try_from(other: String) -> Result<Self, Self::Error> {
		validate_cdata(&other)?;
		Ok(CData(other))
	}
}

impl TryFrom<&str> for CData  {
	type Error = WFError;

	fn try_from(other: &str) -> Result<Self, Self::Error> {
		validate_cdata(other)?;
		Ok(CData(other.to_string()))
	}
}

impl fmt::Display for CData {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		f.write_str(&self.0)
	}
}

/// str which consists only of XML 1.0 Chars.
///
/// [`CDataStr`] corresponds to a (restricted) [`str`]. For a [`String`]-like
/// type with the same restrictions as well as the formal definition of those
/// restrictions, see [`CData`].
///
/// `&CDataStr` can be created from a string literal at compile time using the
/// `xml_cdata` macro from [`rxml_proc`](https://docs.rs/rxml_proc).
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
	#[deprecated(since = "0.4.0", note = "use the TryFrom<> trait implementation  or xml_cdata! macro instead")]
	pub fn from_str<'x>(s: &'x str) -> Result<&'x CDataStr, WFError> {
		s.try_into()
	}

	/// Copy the CDataStr into a new CData
	pub fn to_cdata(&self) -> CData {
		unsafe { CData::from_string_unchecked(self.0.to_string()) }
	}

	/// Construct a CDataStr without checking anything
	///
	/// # Safety
	///
	/// The caller is responsible for ensuring that the passed data is in fact
	/// valid CData.
	pub unsafe fn from_str_unchecked<'x>(s: &'x str) -> &'x CDataStr {
		std::mem::transmute(s)
	}
}

impl Eq for CDataStr {}

impl PartialEq<CDataStr> for &str {
	fn eq(&self, other: &CDataStr) -> bool {
		return *self == &other.0
	}
}

impl PartialEq<&str> for CDataStr {
	fn eq(&self, other: &&str) -> bool {
		return &self.0 == *other
	}
}

impl PartialEq<CDataStr> for str {
	fn eq(&self, other: &CDataStr) -> bool {
		return self == &other.0
	}
}

impl PartialEq<str> for CDataStr {
	fn eq(&self, other: &str) -> bool {
		return &self.0 == other
	}
}

impl Deref for CDataStr {
	type Target = str;

	fn deref(&self) -> &str {
		&self.0
	}
}

impl AsRef<CDataStr> for CDataStr {
	fn as_ref(&self) -> &Self {
		&self
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

impl ToOwned for CDataStr {
	type Owned = CData;

	fn to_owned(&self) -> Self::Owned {
		self.to_cdata()
	}
}

impl From<&CDataStr> for String {
	fn from(other: &CDataStr) -> String {
		other.0.into()
	}
}

impl From<&CDataStr> for SmartString {
	fn from(other: &CDataStr) -> SmartString {
		other.0.into()
	}
}

impl From<&CDataStr> for CData {
	fn from(other: &CDataStr) -> CData {
		unsafe { CData::from_str_unchecked(other) }
	}
}

impl<'x> TryFrom<&'x str> for &'x CDataStr {
	type Error = WFError;

	fn try_from(other: &'x str) -> Result<Self, Self::Error> {
		validate_cdata(other)?;
		Ok(unsafe { std::mem::transmute(other) })
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn split_name_rejects_localname_with_non_namestart_first_char() {
		let nm: Name = "foo:-bar".try_into().unwrap();
		let result = nm.split_name();
		assert!(matches!(result.err().unwrap(), NWFError::InvalidLocalName(_)));
	}

	#[test]
	fn cdatastr_allows_slashes() {
		let _: &CDataStr = "http://www.w3.org/XML/1998/namespace".try_into().unwrap();
	}
}
