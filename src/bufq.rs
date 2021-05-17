use std::io;
use std::borrow::Cow;
use std::collections::VecDeque;

pub const ERR_NODATA: &'static str = "no data in buffer";

/**
# Zero-copy buffered reader for a queue of byte slices.

The BufferQueue acts as a [`std::io::BufRead`] and [`std::io::Read`]. The
data which is to be read from it must be provided using the
[`BufferQueue::push()`] method.

When a buffer which has been pushed to the queue has been read completely, it
is dropped. Users can read arbitrary subsequences of buffers at a time, but
reads will be shortened at buffer edges.

When more data is being read than being pushed, a
[`std::io::ErrorKind::WouldBlock`] I/O error is returned.

## Example

```
use std::io::{Read, ErrorKind};
use rxml::{BufferQueue, Error};
let mut bq = BufferQueue::new();
let mut buf = [0; 4];
// buffer queue has no data, put some into it
bq.push(&b"foo"[..]);
assert_eq!(bq.read(&mut buf).unwrap(), 3);
assert_eq!(&buf[..3], b"foo");
// buffer is now empty, will get WouldBlock
assert!(bq.read(&mut buf).err().unwrap().kind() == ErrorKind::WouldBlock);
// add more data + eof
bq.push(&b"bar"[..]);
bq.push(&b"2342"[..]);
bq.push_eof();
// reads do not go beyond the individual buffers in the queue
assert_eq!(bq.read(&mut buf).unwrap(), 3);
assert_eq!(&buf[..3], b"bar");
assert_eq!(bq.read(&mut buf).unwrap(), 4);
assert_eq!(&buf[..4], b"2342");
// zero-length read on eof
assert_eq!(bq.read(&mut buf).unwrap(), 0);
```
*/
pub struct BufferQueue<'x> {
	q: VecDeque<Cow<'x, [u8]>>,
	offset: usize,
	len: usize,
	eof: bool,
}

impl<'x> BufferQueue<'x> {
	/// Create a new, empty buffer queue.
	pub fn new() -> BufferQueue<'x> {
		BufferQueue{
			q: VecDeque::new(),
			offset: 0,
			len: 0,
			eof: false,
		}
	}

	/// Add the given buffer to the end of the queue.
	///
	/// Any data which can be converted into a `[u8]` Cow can be passed.
	///
	/// # Panics
	///
	/// If [`BufferQueue::push_eof`] has been called.
	pub fn push<'a: 'x, T: Into<Cow<'a, [u8]>>>(&mut self, new: T)
	{
		let new = new.into();
		if self.eof {
			panic!("cannot push behind eof");
		}
		let new_len = match self.len.checked_add(new.len()) {
			None => panic!("length overflow"),
			Some(v) => v,
		};
		self.q.push_back(new);
		self.len = new_len;
	}

	/// Number of bytes which have been enqueued, but not read yet.
	///
	/// Note that this does not count the number of bytes currently owned or
	/// borrowed by the `BufferQueue`; if a buffer has been partially read,
	/// that will be reflected by `len()` even though the memory has not been
	/// released yet.
	pub fn len(&self) -> usize {
		self.len
	}

	/// Push an end-of-file marker to the queue.
	///
	/// After an end-of-file marker has been pushed, it is not possible to
	/// push further buffers. Once the [`BufferQueue`] is then depleted, it
	/// will signal EOF to the caller instead of `WouldBlock`.
	pub fn push_eof(&mut self) {
		self.eof = true;
	}

	/// Return whether the end-of-file marker has been pushed to the queue
	/// already.
	///
	/// [`BufferQueue::push()`] will panic if this function returns true.
	pub fn eof_pushed(&self) -> bool {
		self.eof
	}

	/// Drop all buffered contents immediately
	///
	/// This will effectively reset the length to 0 and cause all future reads
	/// to return either WouldBlock (if [`push_eof`] has not been called yet)
	/// or eof.
	pub fn clear(&mut self) {
		self.q.clear();
		self.len = 0;
	}
}

impl io::Read for BufferQueue<'_> {
	/// Read from the buffer queue.
	///
	/// The read will return bytes up to the next buffer boundary at most;
	/// this means that a short read is **not** an indicator for an impending
	/// end-of-file (this is within the [`std::io::Read`] contract).
	///
	/// When the end-of-file is reached (see [`BufferQueue::push_eof()`]),
	/// zero is returned. When no buffer is available in the queue but the
	/// end-of-file has not been reached yet, a
	/// [`std::io::ErrorKind::WouldBlock`] is returned.
	fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
		let (read, remaining) = {
			let front = match self.q.front() {
				None => {
					if self.eof {
						return Ok(0)
					} else {
						return Err(io::Error::new(io::ErrorKind::WouldBlock, ERR_NODATA))
					}
				},
				Some(v) => v,
			};
			debug_assert!(self.offset < front.len());
			let effective_len = front.len() - self.offset;
			let to_read = std::cmp::min(dst.len(), effective_len);
			for (src, dst) in front[self.offset..(to_read+self.offset)].iter().zip(dst.iter_mut()) {
				*dst = *src;
			};
			self.offset += to_read;
			(to_read, front.len() - self.offset)
		};
		if remaining == 0 {
			self.q.pop_front();
			self.offset = 0;
		}
		self.len -= read;
		Ok(read)
	}
}

impl io::BufRead for BufferQueue<'_> {
	/// Return the current buffer contents
	///
	/// This will only return the contents up to the next buffer boundary,
	/// so callers need to be prepared to see single-byte buffers even though
	/// the end-of-file is not close yet.
	///
	/// If data is currently available and the end-of-file has not been
	/// reached yet, [`std::io::ErrorKind::WouldBlock`] is returned.
	fn fill_buf(&mut self) -> io::Result<&[u8]> {
		match self.q.front() {
			None => if self.eof {
				Ok(&[])
			} else {
				Err(io::Error::new(io::ErrorKind::WouldBlock, ERR_NODATA))
			},
			Some(v) => Ok(&v[self.offset..]),
		}
	}

	/// Skip forward by `amt` bytes
	///
	/// # Panics
	///
	/// If the `amt` bytes is larger than what `fill_buf()` would return.
	fn consume(&mut self, amt: usize) {
		if amt == 0 {
			return;
		}
		let remaining = {
			let front = match self.q.front() {
				None => panic!("attempt to consume beyond end of buffer"),
				Some(v) => v,
			};
			debug_assert!(self.offset < front.len());
			let effective_len = front.len() - self.offset;
			if amt > effective_len {
				panic!("attempt to consume beyond end of buffer");
			}
			self.offset += amt;
			front.len() - self.offset
		};
		if remaining == 0 {
			self.q.pop_front();
			self.offset = 0;
		}
		self.len -= amt;
	}

}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::{Read, BufRead};

	#[test]
	fn bufq_len_grows_with_buffers() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		assert_eq!(bq.len(), 0);
		bq.push(s1.to_vec());
		assert_eq!(bq.len(), 3);
		bq.push(s2.to_vec());
		assert_eq!(bq.len(), 6);
		bq.push(s3.to_vec());
		assert_eq!(bq.len(), 10);
	}

	#[test]
	fn bufq_read_sequentially() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		bq.push(s3.to_vec());
		let mut buf = [0; 3];
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(&buf[..], b"foo");
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(&buf[..], b"bar");
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(&buf[..], b"234");
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 1);
		assert_eq!(&buf[..1], b"2");
	}

	#[test]
	fn bufq_read_limits_at_buffer_edge() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		bq.push(s3.to_vec());
		let mut buf = [0; 4];
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(&buf[..3], b"foo");
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(&buf[..3], b"bar");
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 4);
		assert_eq!(&buf[..], b"2342");
	}

	#[test]
	fn bufq_read_returns_wouldblock_at_end() {
		let s1 = b"foo";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		let mut buf = [0; 4];
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.read(&mut buf[..]).err().unwrap().kind(), io::ErrorKind::WouldBlock);
	}

	#[test]
	fn bufq_read_returns_eof_at_end_if_eof_has_been_set() {
		let s1 = b"foo";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push_eof();
		let mut buf = [0; 4];
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 0);
	}

	#[test]
	fn bufq_returns_eof_flag() {
		let mut bq = BufferQueue::new();
		assert!(!bq.eof_pushed());
		bq.push_eof();
		assert!(bq.eof_pushed());
	}

	#[test]
	#[should_panic(expected = "cannot push behind eof")]
	fn bufq_does_not_allow_pushing_after_eof() {
		let s1 = b"foo";
		let s2 = b"bar";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push_eof();
		bq.push(s2.to_vec());
	}

	#[test]
	fn bufq_read_reduces_length() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		bq.push(s3.to_vec());
		let mut buf = [0; 3];
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.len(), 7);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.len(), 4);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.len(), 1);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 1);
		assert_eq!(bq.len(), 0);
	}

	#[test]
	fn bufq_works_with_fillup_after_depletion() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		let mut buf = [0; 3];
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.len(), 3);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.len(), 0);
		bq.push(s3.to_vec());
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.len(), 1);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 1);
		assert_eq!(bq.len(), 0);
	}

	#[test]
	fn bufq_works_with_intermediate_fillup() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		let mut buf = [0; 3];
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.len(), 3);
		bq.push(s3.to_vec());
		assert_eq!(bq.len(), 7);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.len(), 4);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(bq.len(), 1);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 1);
		assert_eq!(bq.len(), 0);
	}

	#[test]
	fn bufq_consume_reduces_len_and_moves_read_pointer() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		bq.push(s3.to_vec());
		let mut buf = [0; 3];
		assert_eq!(bq.len(), 10);
		bq.consume(3);
		assert_eq!(bq.len(), 7);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(&buf[..], b"bar");
		assert_eq!(bq.len(), 4);
	}

	#[test]
	fn bufq_consume_at_empty_buffer_with_zero_size_is_ok() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		bq.push(s3.to_vec());
		bq.consume(3);
		bq.consume(3);
		bq.consume(4);
		bq.consume(0);
		assert_eq!(bq.len(), 0);
	}

	#[test]
	fn bufq_partial_consume_moves_read_pointer_and_len_correctly() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		bq.push(s3.to_vec());
		bq.consume(2);
		let mut buf = [0; 3];
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 1);
		assert_eq!(&buf[..1], b"o");
		assert_eq!(bq.len(), 7);
		assert_eq!(bq.read(&mut buf[..2]).unwrap(), 2);
		assert_eq!(&buf[..2], b"ba");
		bq.consume(1);
		assert_eq!(bq.len(), 4);
		bq.consume(1);
		assert_eq!(bq.len(), 3);
		assert_eq!(bq.read(&mut buf[..]).unwrap(), 3);
		assert_eq!(&buf[..], b"342");
		assert_eq!(bq.len(), 0);
	}

	#[test]
	#[should_panic(expected = "attempt to consume beyond end of buffer")]
	fn bufq_consume_beyond_buffer_boundaries_panics() {
		let s1 = b"foo";
		let s2 = b"bar";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		assert_eq!(bq.len(), 6);
		bq.consume(4);
	}

	#[test]
	#[should_panic(expected = "attempt to consume beyond end of buffer")]
	fn bufq_consume_beyond_eof_panics() {
		let mut bq = BufferQueue::new();
		bq.push_eof();
		bq.consume(1);
	}

	#[test]
	#[should_panic(expected = "attempt to consume beyond end of buffer")]
	fn bufq_consume_with_empty_buffer_panics() {
		let mut bq = BufferQueue::new();
		bq.consume(1);
	}

	#[test]
	fn bufq_zero_sized_consume_at_eof_is_valid() {
		let mut bq = BufferQueue::new();
		bq.push_eof();
		bq.consume(0);
	}

	#[test]
	fn bufq_zero_sized_consume_with_empty_buffer_is_valid() {
		let mut bq = BufferQueue::new();
		bq.consume(0);
	}

	#[test]
	fn bufq_fill_buf_returns_current_front_buffer() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		bq.push(s3.to_vec());
		assert_eq!(bq.fill_buf().unwrap(), b"foo");
	}

	#[test]
	fn bufq_fill_buf_does_not_consume() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		bq.push(s3.to_vec());
		assert_eq!(bq.fill_buf().unwrap(), b"foo");
		assert_eq!(bq.fill_buf().unwrap(), b"foo");
	}

	#[test]
	fn bufq_fill_buf_works_with_consume() {
		let s1 = b"foo";
		let s2 = b"bar";
		let s3 = b"2342";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push(s2.to_vec());
		bq.push(s3.to_vec());
		assert_eq!(bq.fill_buf().unwrap(), b"foo");
		bq.consume(1);
		assert_eq!(bq.fill_buf().unwrap(), b"oo");
		bq.consume(2);
		assert_eq!(bq.fill_buf().unwrap(), b"bar");
		bq.consume(2);
		assert_eq!(bq.fill_buf().unwrap(), b"r");
		bq.consume(1);
		assert_eq!(bq.fill_buf().unwrap(), b"2342");
	}

	#[test]
	fn bufq_fill_buf_at_eof_returns_empty_buffer() {
		let s1 = b"foo";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push_eof();
		bq.consume(3);
		assert_eq!(bq.fill_buf().unwrap(), b"");
	}

	#[test]
	fn bufq_fill_buf_with_empty_buffer_returns_wouldblock() {
		let s1 = b"foo";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.consume(3);
		assert_eq!(bq.fill_buf().err().unwrap().kind(), io::ErrorKind::WouldBlock);
	}

	#[test]
	fn bufq_empty_after_clear() {
		let s1 = b"foo";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.clear();
		assert_eq!(bq.len(), 0);
		assert_eq!(bq.fill_buf().err().unwrap().kind(), io::ErrorKind::WouldBlock);
	}

	#[test]
	fn bufq_eof_after_clear_with_push_eof_before_clear() {
		let s1 = b"foo";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.push_eof();
		bq.clear();
		assert_eq!(bq.len(), 0);
		assert_eq!(bq.fill_buf().unwrap(), b"");
	}

	#[test]
	fn bufq_eof_after_clear_with_push_eof_after_clear() {
		let s1 = b"foo";
		let mut bq = BufferQueue::new();
		bq.push(s1.to_vec());
		bq.clear();
		bq.push_eof();
		assert_eq!(bq.len(), 0);
		assert_eq!(bq.fill_buf().unwrap(), b"");
	}
}
