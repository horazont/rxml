use std::io;
use crate::error::{Result, Error};
use crate::selectors::CharSelector;

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

	#[inline]
	pub fn to_char(&self) -> char {
		self.ch
	}

	#[inline]
	pub fn as_str<'x>(&'x self) -> &'x str {
		AsRef::<str>::as_ref(self)
	}

	#[inline]
	pub fn len(&self) -> usize {
		self.nbytes as usize
	}
}

impl AsRef<[u8]> for Utf8Char {
	fn as_ref<'x>(&'x self) -> &'x [u8] {
		&self.bytes[..self.nbytes as usize]
	}
}

impl AsRef<str> for Utf8Char {
	fn as_ref<'x>(&'x self) -> &'x str {
		#[cfg(debug_assertions)]
		{
			if !std::str::from_utf8(&self.bytes[..self.nbytes as usize]).is_ok() {
				panic!("invalid utf8 sequence in Utf8Char: {:?}", self)
			}
		}
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

	/// Read all codepoints into a String until the end of file or an error
	/// occurs.
	///
	/// The string is always returned, even on error. The result is ok only on
	/// EOF.
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

/**
# Streaming UTF-8 decoder

Decode UTF-8 from a [`std::io::Read`] and emit individual [`Utf8Char`]s.

**Note:** While the [`DecodingReader`] can work with any [`std::io::Read`], it
is highly recommended to **use a reader with an internal buffer** (such as
[`std::io::BufReader`]), as the decoder reads the source byte-by-byte.

To read codepoints from the reader, use [`DecodingReader::read()`].

It is possible to resume a reader which has previously failed with an error,
but not recommended or guaranteed.
*/
pub struct DecodingReader<T: io::Read + Sized> {
	backend: T,
	buf: [u8; 4],
	buflen: usize,
	accum: u32,
	cont_mask: u8,
	seqlen: u8,
}

impl<T: io::Read + Sized> DecodingReader<T> {
	/// Create a new decoding reader
	pub fn new(r: T) -> Self {
		Self{
			backend: r,
			buf: [0; 4],
			buflen: 0,
			seqlen: 0,
			accum: 0,
			cont_mask: 0,
		}
	}

	/// Consume the reader and return the backing Read
	///
	/// **Warning:** The [`DecodingReader`] buffers a small amount of data
	/// (up to four bytes) while decoding UTF-8 sequences. That data is lost
	/// when using this function.
	pub fn into_inner(self) -> T {
		self.backend
	}

	fn reset(&mut self) {
		self.seqlen = 0;
		self.buflen = 0;
	}

	fn feed(&mut self, c: u8) -> Result<Option<Utf8Char>> {
		debug_assert!(self.buflen < 4);
		if self.seqlen == 0 {
			// new char, analyze starter
			let (raw, len, required_cont_mask) = match c {
				0x00..=0x7fu8 => {
					return Ok(Some(unsafe { Utf8Char::from_ascii_unchecked(c) }));
				},
				// note that 0xc0 and 0xc1 are a invalid start bytes because that is still ascii range
				0xc2..=0xdfu8 => {
					((c & 0x1f) as u32, 2u8, 0xffu8)
				},
				0xe0..=0xefu8 => {
					((c & 0x0f) as u32, 3u8, if c == 0xe0 { 0x20u8 } else { 0xffu8 })
				},
				0xf0..=0xf7u8 => {
					((c & 0x07) as u32, 4u8, if c == 0xf0 { 0x30u8 } else { 0xffu8 })
				},
				_ => return Err(Error::InvalidStartByte(c)),
			};
			self.accum = raw;
			self.seqlen = len;
			self.buf = [c, 0, 0, 0];
			self.accum = raw;
			self.buflen = 1;
			self.cont_mask = required_cont_mask;
			Ok(None)
		} else {
			if c & 0xc0 != 0x80 || c & self.cont_mask == 0 {
				self.reset();
				return Err(Error::InvalidContByte(c));
			}
			self.cont_mask = 0xff;
			self.accum = (self.accum << 6) | ((c & 0x3f) as u32);
			self.buf[self.buflen] = c;
			self.buflen += 1;
			if self.seqlen as usize == self.buflen {
				match std::char::from_u32(self.accum) {
					None => {
						self.reset();
						Err(Error::InvalidChar(self.accum))
					},
					Some(ch) => {
						let utf8ch = Utf8Char{
							bytes: self.buf,
							nbytes: self.seqlen,
							ch: ch
						};
						self.reset();
						Ok(Some(utf8ch))
					},
				}
			} else {
				Ok(None)
			}
		}
	}

	/// Get a reference to the backing Read.
	pub fn get_ref(&self) -> &T {
		&self.backend
	}

	/// Get a reference to the backing Read.
	pub fn get_mut(&mut self) -> &mut T {
		&mut self.backend
	}
}

impl<T: io::Read + Sized> std::borrow::Borrow<T> for DecodingReader<T> {
	fn borrow(&self) -> &T {
		&self.backend
	}
}

impl<T: io::Read + Sized> std::borrow::BorrowMut<T> for DecodingReader<T> {
	fn borrow_mut(&mut self) -> &mut T {
		&mut self.backend
	}
}

impl<T: io::Read + Sized> CodepointRead for DecodingReader<T> {
	/// Decode a single codepoint and return it
	///
	/// If UTF-8 decoding fails, an error is returned. If the data source
	/// reports an [`std::io::Error`] that error is also passed through.
	///
	/// I/O errors are resumable, which means that you can call `read()` again
	/// after it has returned an I/O error.
	fn read(&mut self) -> Result<Option<Utf8Char>> {
		let mut buf = [0u8; 1];
		loop {
			if self.backend.read(&mut buf[..])? == 0 {
				if self.seqlen > 0 {
					// in the middle of a sequence
					return Err(Error::io(io::Error::new(io::ErrorKind::UnexpectedEof, "eof in utf-8 sequence")));
				}
				// eof
				return Ok(None)
			}
			match self.feed(buf[0])? {
				Some(utf8ch) => return Ok(Some(utf8ch)),
				None => (),
			}
		}
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
	let mut remaining = limit;
	while remaining > 0 {
		let utf8ch = match r.read()? {
			None => return Ok(Endpoint::Eof),
			Some(ch) => ch,
		};
		let ch = utf8ch.to_char();
		if !selector.select(ch) {
			return Ok(Endpoint::Delimiter(utf8ch))
		}
		into.push_str(utf8ch.as_ref());
		remaining = remaining.checked_sub(utf8ch.as_str().len()).unwrap_or(0)
	}
	Ok(Endpoint::Limit)
}

pub fn skip_matching<'r, 's, R: CodepointRead, S: CharSelector>(
	r: &'r mut R,
	selector: &'s S,
	) -> (usize, Result<Endpoint>)
{
	let mut count = 0;
	loop {
		let utf8ch = match r.read() {
			Err(e) => return (count, Err(e)),
			Ok(None) => return (count, Ok(Endpoint::Eof)),
			Ok(Some(ch)) => ch,
		};
		let ch = utf8ch.to_char();
		if !selector.select(ch) {
			return (count, Ok(Endpoint::Delimiter(utf8ch)))
		}
		count += 1;
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::selectors::AllChars;
	use crate::bufq;

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
	fn decoding_reader_rejects_utf8_which_is_noncanonical_two_byte_sequence() {
		// found by afl
		for first in 0xc0..=0xc1 {
			let mut src = &[first, b'\xa5'][..];
			let mut r = DecodingReader::new(&mut src);
			let (_v, err) = r.read_all();
			match err {
				Err(Error::InvalidStartByte(..)) => Ok(()),
				Err(e) => Err(e),
				Ok(()) => panic!("expected error"),
			}.unwrap(); // let this panic usefully on mis-match
		}
	}

	#[test]
	fn decoding_reader_rejects_utf8_which_is_noncanonical_three_byte_sequence() {
		let mut src = &b"\xe0\x82"[..];
		let mut r = DecodingReader::new(&mut src);
		let (_, err) = r.read_all();
		match err {
			Err(Error::InvalidContByte(..)) => Ok(()),
			Err(e) => Err(e),
			Ok(()) => panic!("expected error"),
		}.unwrap(); // let this panic usefully on mis-match
	}

	#[test]
	fn decoding_reader_rejects_utf8_which_is_noncanonical_four_byte_sequence() {
		let mut src = &b"\xf0\x82"[..];
		let mut r = DecodingReader::new(&mut src);
		let (_v, err) = r.read_all();
		match err {
			Err(Error::InvalidContByte(..)) => Ok(()),
			Err(e) => Err(e),
			Ok(()) => panic!("expected error"),
		}.unwrap(); // let this panic usefully on mis-match
	}

	#[test]
	fn decoding_reader_can_parse_segmented_utf8_sequence() {
		let seq = &b"\xf0\x9f\x8e\x89"[..];
		let mut bq = bufq::BufferQueue::new();
		bq.push(seq[..2].to_vec());
		bq.push(seq[2..4].to_vec());
		bq.push_eof();
		let mut r = DecodingReader::new(&mut bq);
		let (v, err) = r.read_all();
		err.unwrap();
		assert_eq!(v, "🎉");
	}

	#[test]
	fn decoding_reader_can_resume_from_wouldblock_during_utf8_sequence() {
		let seq = &b"\xf0\x9f\x8e\x89"[..];
		let mut bq = bufq::BufferQueue::new();
		bq.push(seq[..2].to_vec());
		let mut r = DecodingReader::new(&mut bq);
		let (v, err) = r.read_all();
		assert_eq!(v, "");
		assert!(matches!(err.err().unwrap(), Error::IO(ioe) if ioe.kind() == io::ErrorKind::WouldBlock));
		r.get_mut().push(seq[2..4].to_vec());
		let (v, err) = r.read_all();
		assert_eq!(v, "🎉");
		assert!(matches!(err.err().unwrap(), Error::IO(ioe) if ioe.kind() == io::ErrorKind::WouldBlock));
	}

	#[test]
	fn decoding_reader_fuzz() {
		let mut src = &b"?\xf0\xa4\xa4\xa4\x9e\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4\xa4"[..];
		let mut r = DecodingReader::new(&mut src);
		let (_, err) = r.read_all();
		match err {
			Err(Error::InvalidStartByte(158)) => Ok(()),
			Err(e) => Err(e),
			Ok(()) => panic!("expected error"),
		}.unwrap(); // let this panic usefully on mis-match
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
	fn read_validated_limits_by_bytes() {
		let mut s1 = &b"f\xc3\xb6\xc3\xb6b\xc3\xa4r2342"[..];
		let mut r = DecodingReader::new(&mut s1);
		let mut out = String::new();
		let result = read_validated(&mut r, &AllChars(), 6, &mut out);
		assert!(matches!(result.unwrap(), Endpoint::Limit));
		assert_eq!(out, "fööb".to_string());
	}

	#[test]
	fn read_validated_may_exceed_limit_slightly_for_utf8_sequence() {
		let mut s1 = &b"f\xc3\xb6\xc3\xb6b\xc3\xa4r2342"[..];
		let mut r = DecodingReader::new(&mut s1);
		let mut out = String::new();
		let result = read_validated(&mut r, &AllChars(), 4, &mut out);
		assert!(matches!(result.unwrap(), Endpoint::Limit));
		assert_eq!(out, "föö".to_string());
		assert_eq!(out.len(), 5);
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

	#[test]
	fn exhaustive_utf8_test() {
		let mut buf = [0u8; 4];
		for chu32 in 0..=0x10ffffu32 {
			if let Some(ch) = std::char::from_u32(chu32) {
				let len = ch.encode_utf8(&mut buf[..]).len();
				let mut src = &buf[..len];
				let mut r = DecodingReader::new(&mut src);
				match r.read() {
					Err(e) => panic!("decoding of U+{:x} {:?} failed: {:?}", chu32, &buf[..len], e),
					Ok(Some(v)) => (),
					Ok(None) => panic!("decoding of U+{:x} {:?} incorrectly claims eof", chu32, &buf[..]),
				}
			}
		}
	}
}
