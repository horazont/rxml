use std::io;
use std::cmp;
use std::ptr;

/* trait CodepointBufRead {
	fn fill_buf(&mut self) -> io::Result<&str>;
	fn consume(&mut self, n: usize);

	fn read(&mut self, buf: &mut str) -> io::Result<usize> {
		match self.fill_buf() {
			Ok(avail) => {
				let to_read = if buf.len() > avail.len() {
					avail.len()
				} else {
					buf.len()
				};
				buf[..to_read].copy_from_slice(avail[..to_read]);
				self.consume(to_read);
				Ok(to_read)
			},
			Err(e) => Err(e),
		}
	}
} */

pub fn is_start_byte(b: u8) -> bool {
	return (b & 0x80u8 == 0) || (b & 0xc0 == 0xc0)
}

pub fn find_previous_boundary(src: &str, start_at: usize) -> usize {
	let mut curr = start_at;
	loop {
		// note: str.is_char_boundary() returns true for index 0, so this
		// loop will always terminate cleanly.
		if src.is_char_boundary(curr) {
			return curr;
		}
		curr -= 1;
	}
}

/// Safely copy as much from `src` as possible into `dest`. Return the number
/// of bytes copied.
///
/// This function preserves the invariant that str is utf8 encoded, hence it
/// is possible that the return value is less than the length of both strings
/// (specifically, if `min(dest.len(), src.len())` points inside an utf8
/// sequence)
pub fn safe_str_slice_copy(dest: &mut str, src: &str) -> usize {
	let min_len = cmp::min(dest.len(), src.len());
	let to_copy = find_previous_boundary(src, min_len);
	unsafe {
		let dest_u8 = dest.as_mut_ptr();
		let src_u8 = src.as_ptr();
		// this is safe, because dest and src cannot point to the same place
		// (borrow checking)
		ptr::copy_nonoverlapping(src_u8, dest_u8, to_copy);
	}
	to_copy
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn is_start_byte_plain_ascii_is_start() {
		for i in (0..0x80u8) {
			assert!(is_start_byte(i))
		}
	}

	#[test]
	fn is_start_byte_detects_multibyte_start() {
		let s1 = b"\xf0\x9f\x8e\x89";
		let s2 = b"\xc3\xa4";
		assert!(is_start_byte(s1[0]));
		assert!(is_start_byte(s2[0]));
	}

	#[test]
	fn is_start_byte_rejects_interior_bytes() {
		let s1 = b"\xf0\x9f\x8e\x89";
		let s2 = b"\xc3\xa4";
		assert!(!is_start_byte(s1[1]));
		assert!(!is_start_byte(s1[2]));
		assert!(!is_start_byte(s1[3]));
		assert!(!is_start_byte(s2[1]));
	}

	#[test]
	fn find_previous_boundary_returns_input_on_boundary() {
		let s1 = "abc";
		let s2 = "fööbär";
		let s3 = "Party 🎉!";

		fn test(s: &str) {
			for (offset, ch) in s.char_indices() {
				assert_eq!(find_previous_boundary(s, offset), offset);
			}
		}
	}

	#[test]
	fn find_previous_boundary_backsteps_as_needed() {
		assert_eq!(find_previous_boundary("äüö", 1), 0);
		assert_eq!(find_previous_boundary("äüö", 3), 2);
		assert_eq!(find_previous_boundary("Party 🎉!", 6), 6);
		assert_eq!(find_previous_boundary("Party 🎉!", 7), 6);
		assert_eq!(find_previous_boundary("Party 🎉!", 8), 6);
		assert_eq!(find_previous_boundary("Party 🎉!", 9), 6);
		assert_eq!(find_previous_boundary("Party 🎉!", 10), 10);
	}
}
