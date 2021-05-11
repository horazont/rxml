#[macro_use]
extern crate afl;
extern crate xmppxml;

use std::fmt;
use std::io;
use std::io::BufRead;

fn main() {
    fuzz!(|data: &[u8]| {
		let mut lexer = xmppxml::Lexer::new();
		let mut buffered = io::BufReader::new(data);

		loop {
			match lexer.lex(&mut buffered) {
				Err(_) => return,
				Ok(None) => return,
				Ok(Some(_)) => (),
			}
		}
    });
}
