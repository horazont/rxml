#[macro_use]
extern crate afl;
extern crate rxml;

use std::io;
use rxml::EventRead;

fn lex_chunked<'c, 'cc>(chunks: &'c [&'cc [u8]]) -> rxml::Result<usize> {
	let mut nevents = 0;
	let mut parser = rxml::FeedParser::new();

	for chunk in chunks {
		parser.feed(*chunk);

		match parser.read_all(|_| { nevents += 1 }) {
			Err(rxml::Error::IO(ioerr)) if ioerr.kind() == io::ErrorKind::WouldBlock => (),
			Err(e) => return Err(e),
			Ok(()) => panic!("eof reached before eof"),
		}
	}

	parser.feed_eof();
	parser.read_all(|_| { nevents += 1})?;
	Ok(nevents)
}

fn main() {
    fuzz!(|data: &[u8]| {
		let mut had_any_err = false;
		let mut had_all_err = true;
		let mut chunks = Vec::<&[u8]>::new();
		let zero = &b"\0"[..];
		for chunk in data.split(|b| { *b == b'\0' }) {
			if chunk.len() == 0 {
				chunks.push(zero)
			} else {
				chunks.push(chunk)
			}
		}
		match lex_chunked(&chunks) {
			Ok(_) => {
				had_all_err = false;
			},
			Err(_) => {
				had_any_err = true;
			},
		}
		let buf = chunks.join(&b""[..]);
		match lex_chunked(&[&buf]) {
			Ok(_) => {
				had_all_err = false;
			},
			Err(_) => {
				had_any_err = true;
			},
		}

		if had_any_err && !had_all_err {
			panic!("error state depends on chunking")
		}
    });
}
