use super::ranges::ByteSelect;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Endbyte {
	Eof,
	Limit,
	Delimiter(u8),
}

fn find_first_not<B: ByteSelect>(src: &[u8], s: &B) -> Option<usize> {
	for i in 0..src.len() {
		// range check via for loop
		let b = {
			#[cfg(debug_assertions)]
			{
				src[i]
			}
			#[cfg(not(debug_assertions))]
			*unsafe {
				src.get_unchecked(i)
			}
		};
		if !s.select(b) {
			return Some(i)
		}
	}
	None
}

pub fn read_validated_bytes<B: ByteSelect>(
	r: &mut &[u8],
	selector: &B,
	limit: usize,
	into: &mut Vec<u8>,
	) -> Endbyte
{
	let end_pos  = match find_first_not(*r, selector) {
		None => r.len().min(limit),
		Some(p) => p.min(limit),
	};
	let (prefix, delim_suffix) = r.split_at(end_pos);
	into.extend_from_slice(prefix);
	if delim_suffix.len() > 0 {
		// => we have a delimiter or the length limit ... which of the two is it? easy! we check
		// (if we just happened to run in the length limit *at* the delimiter, that's no correctness issue and potentially saves another call)
		let b = delim_suffix[0];
		if !selector.select(b) {
			*r = &delim_suffix[1..];
			Endbyte::Delimiter(b)
		} else {
			*r = delim_suffix;
			Endbyte::Limit
		}
	} else {
		*r = &[];
		Endbyte::Eof
	}
}

pub fn skip_matching_bytes<B: ByteSelect>(
	r: &mut &[u8],
	selector: &B,
	) -> (usize, Endbyte)
{
	let end_pos  = match find_first_not(*r, selector) {
		None => r.len(),
		Some(p) => p,
	};
	let (_, delim_suffix) = r.split_at(end_pos);
	if delim_suffix.len() > 0 {
		// => we have a delimiter or the length limit ... which of the two is it? easy! we check
		// (if we just happened to run in the length limit *at* the delimiter, that's no correctness issue and potentially saves another call)
		let b = delim_suffix[0];
		if !selector.select(b) {
			*r = &delim_suffix[1..];
			(end_pos, Endbyte::Delimiter(b))
		} else {
			*r = delim_suffix;
			(end_pos, Endbyte::Limit)
		}
	} else {
		*r = &[];
		(end_pos, Endbyte::Eof)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::lexer::ranges::AnyByte;

	#[test]
	fn read_validated_bytes_limits() {
		let mut s1 = &b"foobar2342"[..];
		let mut out = Vec::new();
		let result = read_validated_bytes(&mut s1, &AnyByte(), 6, &mut out);
		assert!(matches!(result, Endbyte::Limit));
		assert_eq!(out, b"foobar".to_vec());
		assert_eq!(s1.len(), 4);
	}

	#[test]
	fn read_validated_bytes_limits_by_bytes() {
		let mut s1 = &b"f\xc3\xb6\xc3\xb6b\xc3\xa4r2342"[..];
		let mut out = Vec::new();
		let result = read_validated_bytes(&mut s1, &AnyByte(), 6, &mut out);
		assert!(matches!(result, Endbyte::Limit));
		assert_eq!(out, "fööb".as_bytes().to_vec());
		assert_eq!(s1.len(), 7);
	}

	#[test]
	fn read_validated_bytes_does_not_care_about_utf8() {
		let mut s1 = &b"f\xc3\xb6\xc3\xb6b\xc3\xa4r2342"[..];
		let mut out = Vec::new();
		let result = read_validated_bytes(&mut s1, &AnyByte(), 4, &mut out);
		match result {
			Endbyte::Limit => (),
			other => panic!("unexpected result: {:?}", other),
		}
		assert_eq!(out, b"f\xc3\xb6\xc3".to_vec());
		assert_eq!(out.len(), 4);
		assert_eq!(s1.len(), 9);
	}

	#[test]
	fn read_validated_bytes_handles_eof() {
		let mut s1 = &b"foobar2342"[..];
		let mut out = Vec::new();
		let result = read_validated_bytes(&mut s1, &AnyByte(), 128, &mut out);
		match result {
			Endbyte::Eof => (),
			other => panic!("unexpected result: {:?}", other),
		}
		assert_eq!(out, b"foobar2342".to_vec());
		assert_eq!(s1.len(), 0);
	}

	#[test]
	fn read_validated_bytes_returns_delimiter() {
		let mut s1 = &b"fffnord"[..];
		let mut out = Vec::new();
		let result = read_validated_bytes(&mut s1, &b'f', 128, &mut out);
		match result {
			Endbyte::Delimiter(b) if b == b'n' => (),
			other => panic!("unexpected result: {:?}", other),
		}
		assert_eq!(out, b"fff".to_vec());
		assert_eq!(s1.len(), 3);
	}

	#[test]
	fn skip_matching_bytes_handles_eof() {
		let mut s1 = &b"foobar2342"[..];
		let (n, result) = skip_matching_bytes(&mut s1, &AnyByte());
		match result {
			Endbyte::Eof => (),
			other => panic!("unexpected result: {:?}", other),
		}
		assert_eq!(n, 10);
		assert_eq!(s1.len(), 0);
	}

	#[test]
	fn skip_matching_bytes_returns_delimiter() {
		let mut s1 = &b"fffnord"[..];
		let (n, result) = skip_matching_bytes(&mut s1, &b'f');
		match result {
			Endbyte::Delimiter(b) if b == b'n' => (),
			other => panic!("unexpected result: {:?}", other),
		}
		assert_eq!(n, 3);
		assert_eq!(s1.len(), 3);
	}
}
