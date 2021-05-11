mod error;
pub mod lexer;

pub use lexer::Lexer;
pub use lexer::DecodingReader;

/* impl BufferQueue {
	/// EOF the stream after all buffered bytes have been read.
	pub fn close(&mut self) -> Result<()> {
		Ok(())
	}

	/// Feed more bytes into the buffer.
	pub fn feed(&mut self, data: &[u8]) -> Result<()> {
		Ok(())
	}

	/// After the current bytes have been read, the given error will be
	/// returned to the caller indefinitely.
	pub fn feed_error(&mut self, err: io::Error) -> Result<()> {
		Ok(())
	}

	// Current amount of buffered data
	pub fn len() -> usize {

	}
}

impl PeekableRead for BufferQueue {
	fn peek(&self, dest: &mut [u8]) -> Result<usize, io::Error> {
		Err(io::Error::new(io::ErrorKind::WouldBlock, "buffer is empty"))
	}

	fn read(&self, dest: &mut [u8]) -> Result<usize, io::Error> {
		Err(io::Error::new(io::ErrorKind::WouldBlock, "buffer is empty"))
	}

	fn skip(&self, n: usize) -> Result<(), io::Error> {
		Err(io::Error::new(io::ErrorKind::WouldBlock, "buffer is empty"))
	}
}

impl Parser {
	/// WouldBlock -> backend needs to provide more data.
	pub fn process() -> Result<()> {
		// 1. read stuff
		// 2. on read error, propagate, but keep consistent state.
	}
} */
