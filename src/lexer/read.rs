use std::io;
use crate::error::{Result, Error};

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Utf8Char{
	ch: char,
	bytes: [u8; 4],
	nbytes: u8,
}

impl Utf8Char {
	pub unsafe fn from_ascii_unchecked(b: u8) -> Utf8Char {
		debug_assert!(b < 0x80u8);
		Utf8Char{
			bytes: [b, 0, 0, 0],
			nbytes: 1,
			ch: b as char,
		}
	}

	pub fn from_ascii(b: u8) -> Option<Utf8Char> {
		if b >= 0x80u8 {
			None
		} else {
			Some(unsafe { Self::from_ascii_unchecked(b) })
		}
	}

	pub fn from_char(c: char) -> Utf8Char {
		let mut buf: [u8; 4] = [0, 0, 0, 0];
		let len = c.encode_utf8(&mut buf).len();
		debug_assert!(len <= 4);
		Utf8Char{
			bytes: buf,
			nbytes: len as u8,
			ch: c,
		}
	}

	pub fn to_char(&self) -> char {
		self.ch
	}

	pub fn as_str<'x>(&'x self) -> &'x str {
		AsRef::<str>::as_ref(self)
	}
}

impl AsRef<[u8]> for Utf8Char {
	fn as_ref<'x>(&'x self) -> &'x [u8] {
		&self.bytes[..self.nbytes as usize]
	}
}

impl AsRef<str> for Utf8Char {
	fn as_ref<'x>(&'x self) -> &'x str {
		debug_assert!(std::str::from_utf8(&self.bytes[..self.nbytes as usize]).is_ok());
		unsafe { std::str::from_utf8_unchecked(&self.bytes[..self.nbytes as usize]) }
	}
}

impl From<Utf8Char> for char {
	fn from(c: Utf8Char) -> char {
		c.ch
	}
}

pub trait CodepointRead {
	fn read(&mut self) -> Result<Option<Utf8Char>>;

	fn read_all(&mut self) -> (String, Result<()>) {
		let mut result = String::new();
		loop {
			match self.read() {
				Ok(Some(ch)) => result.push_str(AsRef::<str>::as_ref(&ch)),
				Ok(None) => return (result, Ok(())),
				Err(e) => return (result, Err(e)),
			}
		}
	}
}

pub struct DecodingReader<'x, T: io::BufRead + ?Sized> {
	backend: &'x mut T,
}

impl<'x, T: io::BufRead + ?Sized> DecodingReader<'x, T> {
	pub fn new(r: &'x mut T) -> Self {
		Self{
			backend: r,
		}
	}
}

impl<'x, T: io::BufRead + ?Sized> CodepointRead for DecodingReader<'x, T> {
	fn read(&mut self) -> Result<Option<Utf8Char>> {
		let mut backing = self.backend.fill_buf()?;
		let mut out: [u8; 4] = [0, 0, 0, 0];
		let mut out_offset = 0usize;
		let starter = match backing.first() {
			None => return Ok(None),
			Some(ch) => *ch,
		};
		if starter & 0x80 == 0 {
			// ascii
			self.backend.consume(1);
			return Ok(Some(unsafe { Utf8Char::from_ascii_unchecked(starter) }));
		}
		out[out_offset] = starter;
		out_offset += 1;

		let (mut raw, len) = if starter & 0xe0 == 0xc0 {
			((starter & 0x1f) as u32, 2usize)
		} else if starter & 0xf0 == 0xe0 {
			((starter & 0x0f) as u32, 3usize)
		} else if starter & 0xf8 == 0xf0 {
			((starter & 0x07) as u32, 4usize)
		} else {
			return Err(Error::InvalidStartByte(starter))
		};

		let mut more = len - 1;
		let mut offset = 1;
		while more > 0 {
			if backing.len() <= offset {
				self.backend.consume(offset);
				offset = 0;
				backing = self.backend.fill_buf()?;
			}
			let next = *match backing[offset..].first() {
				Some(b) => b,
				None => {
					self.backend.consume(offset);
					return Err(Error::IO(io::Error::new(io::ErrorKind::UnexpectedEof, "eof within utf-8 sequence")));
				},
			};
			if next & 0xc0 != 0x80 {
				self.backend.consume(offset);
				return Err(Error::InvalidContByte(next));
			}
			out[out_offset] = next;
			raw = (raw << 6) | ((next & 0x3f) as u32);
			offset += 1;
			out_offset += 1;
			more -= 1;
		}
		if offset > 0 {
			self.backend.consume(offset);
		}

		match std::char::from_u32(raw) {
			None => Err(Error::InvalidChar(raw)),
			Some(ch) => Ok(Some(Utf8Char{
				bytes: out,
				nbytes: len as u8,
				ch: ch
			})),
		}
	}
}

pub trait CharSelector {
	fn select(&self, c: char) -> bool;
}

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

pub enum Endpoint {
	Eof,
	Limit,
	Delimiter(Utf8Char),
}

pub fn read_validated<'r, 's, R: CodepointRead, S: CharSelector>(
	r: &'r mut R,
	selector: &'s S,
	limit: usize,
	into: &mut String,
	) -> Result<Endpoint>
{
	for _ in 0..limit {
		let utf8ch = match r.read()? {
			None => return Ok(Endpoint::Eof),
			Some(ch) => ch,
		};
		let ch = utf8ch.to_char();
		if !selector.select(ch) {
			return Ok(Endpoint::Delimiter(utf8ch))
		}
		into.push_str(utf8ch.as_ref());
	}
	Ok(Endpoint::Limit)
}

pub fn skip_matching<'r, 's, R: CodepointRead, S: CharSelector>(
	r: &'r mut R,
	selector: &'s S,
	) -> Result<Endpoint>
{
	loop {
		let utf8ch = match r.read()? {
			None => return Ok(Endpoint::Eof),
			Some(ch) => ch,
		};
		let ch = utf8ch.to_char();
		if !selector.select(ch) {
			return Ok(Endpoint::Delimiter(utf8ch))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn decoding_reader_can_read_ascii() {
		let mut s1 = &b"foobar2342"[..];
		let mut r = DecodingReader::new(&mut s1);
		let (v, err) = r.read_all();
		err.unwrap();
		assert_eq!(v, "foobar2342".to_string());
	}

	#[test]
	fn decoding_reader_can_read_utf8() {
		let mut s1 = &b"f\xc3\xb6\xc3\xb6b\xc3\xa4r2342\xf0\x9f\x8e\x89"[..];
		let mut r = DecodingReader::new(&mut s1);
		let (v, err) = r.read_all();
		err.unwrap();
		assert_eq!(v, "fööbär2342🎉".to_string());
	}

	#[test]
	fn decoding_reader_can_read_utf8_from_heavily_chunked_stream() {
		let s1 = &b"f\xc3\xb6\xc3\xb6b\xc3\xa4r2342\xf0\x9f\x8e\x89"[..];
		let mut chunked = io::BufReader::with_capacity(1, s1);
		let mut r = DecodingReader::new(&mut chunked);
		let (v, err) = r.read_all();
		err.unwrap();
		assert_eq!(v, "fööbär2342🎉".to_string());
	}

	#[test]
	fn decoding_reader_rejects_invalid_start_byte() {
		let s1 = &b"f\xff"[..];
		let mut chunked = io::BufReader::with_capacity(1, s1);
		let mut r = DecodingReader::new(&mut chunked);
		let (v, err) = r.read_all();
		assert!(matches!(err, Err(Error::InvalidStartByte(0xffu8))));
		assert_eq!(v, "f".to_string());
	}

	#[test]
	fn decoding_reader_rejects_invalid_continuation_byte() {
		let s1 = &b"f\xc3\xff"[..];
		let mut chunked = io::BufReader::with_capacity(1, s1);
		let mut r = DecodingReader::new(&mut chunked);
		let (v, err) = r.read_all();
		assert!(matches!(err, Err(Error::InvalidContByte(0xffu8))));
		assert_eq!(v, "f".to_string());
	}

	#[test]
	fn decoding_reader_reports_premature_eof() {
		let mut s1 = &b"f\xc3"[..];
		let mut r = DecodingReader::new(&mut s1);
		let (v, err) = r.read_all();
		match err {
			Err(Error::IO(e)) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
			Err(e) => Err(e),
			Ok(()) => panic!("expected error"),
		}.unwrap(); // let this panic usefully on mis-match
		assert_eq!(v, "f".to_string());
	}

	#[test]
	fn utf8char_from_char() {
		let ch = Utf8Char::from_char('x');
		assert_eq!(AsRef::<str>::as_ref(&ch), "x");
		assert_eq!(ch.to_char(), 'x');

		let ch = Utf8Char::from_char('ä');
		assert_eq!(AsRef::<str>::as_ref(&ch), "ä");
		assert_eq!(ch.to_char(), 'ä');

		let ch = Utf8Char::from_char('🎉');
		assert_eq!(AsRef::<str>::as_ref(&ch), "🎉");
		assert_eq!(ch.to_char(), '🎉');
	}

	#[test]
	fn utf8char_from_ascii() {
		let ch = Utf8Char::from_ascii(0x20).unwrap();
		assert_eq!(AsRef::<str>::as_ref(&ch), " ");
		assert_eq!(ch.to_char(), ' ');

		let ch = Utf8Char::from_ascii(0x7f).unwrap();
		assert_eq!(AsRef::<str>::as_ref(&ch), "\u{7f}");
		assert_eq!(ch.to_char(), '\u{7f}');
	}

	#[test]
	fn utf8char_from_ascii_rejects_non_ascii() {
		assert!(Utf8Char::from_ascii(0x80).is_none());
	}

	#[test]
	fn decoding_reader_rejects_invalid_char() {
		let mut s1 = &b"\xed\xb0\x80"[..];
		let mut r = DecodingReader::new(&mut s1);
		let (v, err) = r.read_all();
		assert!(matches!(err, Err(Error::InvalidChar(0xdc00u32))));
		assert_eq!(v, "".to_string());
	}

	#[test]
	fn read_validated_limits() {
		let mut s1 = &b"foobar2342"[..];
		let mut r = DecodingReader::new(&mut s1);
		let mut out = String::new();
		let result = read_validated(&mut r, &AllChars(), 6, &mut out);
		assert!(matches!(result.unwrap(), Endpoint::Limit));
		assert_eq!(out, "foobar".to_string());
	}

	#[test]
	fn read_validated_handles_eof() {
		let mut s1 = &b"foobar2342"[..];
		let mut r = DecodingReader::new(&mut s1);
		let mut out = String::new();
		let result = read_validated(&mut r, &AllChars(), 128, &mut out);
		assert!(matches!(result.unwrap(), Endpoint::Eof));
		assert_eq!(out, "foobar2342".to_string());
	}

	#[test]
	fn read_validated_passes_error() {
		let mut s1 = &b"f\xff"[..];
		let mut r = DecodingReader::new(&mut s1);
		let mut out = String::new();
		let result = read_validated(&mut r, &AllChars(), 128, &mut out);
		assert!(matches!(result, Err(Error::InvalidStartByte(0xff))));
		assert_eq!(out, "f".to_string());
	}

	#[test]
	fn read_validated_returns_delimiter() {
		let mut s1 = &b"fffnord"[..];
		let mut r = DecodingReader::new(&mut s1);
		let mut out = String::new();
		let result = read_validated(&mut r, &'f', 128, &mut out);
		assert!(matches!(result.unwrap(), Endpoint::Delimiter(c) if c.to_char() == 'n'));
		assert_eq!(out, "fff".to_string());
	}
}
