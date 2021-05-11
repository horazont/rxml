

trait PeekableRead<'s> {
	fn peek(&'s mut self, min: usize) -> Result<&'s [u8], io::Error>;
	fn read(&'s mut self, dest: &mut [u8]) -> Result<usize, io::Error>;
	fn skip(&'s mut self, n: usize) -> Result<(), io::Error>;
}

struct ByteSource<'a> {
	data: Cow<'a, [u8]>,
	pos: usize,
	has_eof: bool,
}

impl<'s> ByteSource<'s> {
	pub fn new<'a>(data: Cow<'a, [u8]>, has_eof: bool) -> ByteSource<'a> {
		ByteSource{
			data: data,
			pos: 0,
			has_eof: has_eof,
		}
	}

	fn data(&'s self) -> &'s [u8] {
		&self.data[self.pos..]
	}
}




/* impl<'s> PeekableRead<'s> for ByteSource<'s> {
	fn peek(&'s mut self, min: usize) -> Result<&'s [u8], io::Error> {
		let data = self.data();
		if data.len() < min {
			if self.has_eof {
				Err(io::Error::new(io::ErrorKind::UnexpectedEof, "cannot peek beyond EOF"))
			} else {
				Err(io::Error::new(io::ErrorKind::WouldBlock, "ByteSource temporarily depleted"))
			}
		} else {
			Ok(&data[..min])
		}
	}

	fn read(&'s mut self, dest: &mut [u8]) -> Result<usize, io::Error> {
		let data = self.data();
		let to_copy = if dest.len() > data.len() {
			dest.len()
		} else {
			data.len()
		};
		if to_copy == 0 && !self.has_eof {
			return Err(io::Error::new(io::ErrorKind::WouldBlock, "ByteSource temporarily 	depleted"))
		}
		dest.copy_from_slice(&data[..to_copy]);
		self.skip(to_copy).unwrap();
		Ok(to_copy)
	}

	fn skip(&'s mut self, n: usize) -> Result<(), io::Error> {
		let data = self.data();
		if data.len() < n {
			if self.has_eof {
				Err(io::Error::new(io::ErrorKind::UnexpectedEof, "cannot skip beyond EOF"))
			} else {
				Err(io::Error::new(io::ErrorKind::WouldBlock, "ByteSource temporarily 	depleted"))
			}
		} else {
			self.pos += n;
			Ok(())
		}
	}
} */

	/* #[test]
	fn byte_source_peek_does_not_advance_read_pointer() {
		let mut src = ByteSource::new(Cow::from("foobar2342".as_bytes()), false);
		{
			let peek_r = src.peek(2);
			assert!(peek_r.is_ok());
			assert_eq!(&peek_r.unwrap()[..2], "fo".as_bytes());
		}
		{
			let peek_r = src.peek(2);
			assert!(peek_r.is_ok());
			assert_eq!(&peek_r.unwrap()[..2], "fo".as_bytes());
		}
	} */
