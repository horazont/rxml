#[macro_use]
extern crate afl;
extern crate xmppxml;

use std::io;

fn main() {
    fuzz!(|data: &[u8]| {
		let mut lexer = xmppxml::Lexer::new();
		let mut buffered = io::BufReader::new(data);
		let mut reader = xmppxml::DecodingReader::new(&mut buffered);
		let mut adapter = xmppxml::LexerAdapter::new(&mut lexer, &mut reader);
		let mut parser = xmppxml::Parser::new();

		loop {
			match parser.parse(&mut adapter) {
				Err(_) => return,
				Ok(None) => return,
				Ok(Some(_)) => (),
			}
		}
    });
}
