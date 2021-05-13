#[macro_use]
extern crate afl;
extern crate rxml;

use std::io;

fn main() {
    fuzz!(|data: &[u8]| {
		let mut parser = rxml::FeedParser::new();
		parser.feed(data.to_vec());
		parser.feed_eof();

		loop {
			match parser.read() {
				Err(_) => return,
				Ok(None) => return,
				Ok(Some(_)) => (),
			}
		}
    });
}
