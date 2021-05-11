#[macro_use]
extern crate afl;
extern crate xmppxml;

use std::io;

fn main() {
    fuzz!(|data: &[u8]| {
		let mut lexer = xmppxml::Lexer::new();
		let mut buffered = io::BufReader::new(data);
		let mut reader = xmppxml::DecodingReader::new(&mut buffered);

		loop {
			match lexer.lex(&mut reader) {
				Err(_) => return,
				Ok(None) => return,
				Ok(Some(_)) => (),
			}
		}
    });
}
