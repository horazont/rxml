use std::io;

pub mod error;
pub mod lexer;
pub mod parser;
pub mod bufq;

pub use error::{Error, Result};
pub use lexer::{Lexer, DecodingReader};
pub use parser::{Parser, Event, LexerAdapter};
pub use bufq::BufferQueue;

pub struct FeedParser {
	lexer: Lexer,
	parser: Parser,
	bufq: BufferQueue,
}

impl FeedParser {
	pub fn new() -> FeedParser {
		FeedParser{
			lexer: Lexer::new(),
			parser: Parser::new(),
			bufq: BufferQueue::new(),
		}
	}

	pub fn feed(&mut self, data: Vec<u8>) {
		self.bufq.push(data);
	}

	pub fn feed_eof(&mut self) {
		self.bufq.push_eof();
	}

	pub fn buffered(&self) -> usize {
		self.bufq.len()
	}

	pub fn read(&mut self) -> Result<Option<Event>> {
		self.parser.parse(&mut LexerAdapter::new(
			&mut self.lexer,
			&mut DecodingReader::new(&mut self.bufq),
		))
	}

	pub fn feed_all<F>(&mut self, data: Option<Vec<u8>>, mut cb: F) -> Result<bool>
		where F: FnMut(Event) -> ()
	{
		if let Some(data) = data {
			self.feed(data);
		}
		loop {
			match self.read() {
				// at eof
				Ok(None) => return Ok(true),
				// at end of buffer
				Err(Error::IO(ioerr)) if ioerr.kind() == io::ErrorKind::WouldBlock => return Ok(false),
				// other error, propagate
				Err(e) => return Err(e),
				Ok(Some(ev)) => cb(ev),
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn feedparser_can_read_xml_document() {
		let doc = b"<?xml version='1.0'?>\n<root xmlns='urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4' a=\"foo\" b='bar'><child>with some text</child></root>";

		let mut fp = FeedParser::new();
		let mut out = Vec::<Event>::new();
		let result = fp.feed_all(Some(doc.to_vec()), |ev| {
			out.push(ev);
		});
		assert_eq!(result.unwrap(), false);

		{
			let mut iter = out.iter();
			match iter.next().unwrap() {
				Event::StartElement((nsuri, localname), attrs) => {
					assert_eq!(*nsuri.as_ref().unwrap(), "urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4");
					assert_eq!(localname, "root");
					assert_eq!(attrs.len(), 2);
					assert_eq!(attrs.get(&(None, "a".to_string())).unwrap(), "foo");
					assert_eq!(attrs.get(&(None, "b".to_string())).unwrap(), "bar");
				},
				other => panic!("unexpected event: {:?}", other),
			};
			match iter.next().unwrap() {
				Event::StartElement((nsuri, localname), attrs) => {
					assert_eq!(*nsuri.as_ref().unwrap(), "urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4");
					assert_eq!(localname, "child");
					assert_eq!(attrs.len(), 0);
				},
				other => panic!("unexpected event: {:?}", other),
			};
			assert_eq!(*iter.next().unwrap(), Event::Text("with some text".to_string()));
			assert_eq!(*iter.next().unwrap(), Event::EndElement);
			assert_eq!(*iter.next().unwrap(), Event::EndElement);
		}

		fp.feed_eof();
		let result = fp.feed_all(None, |ev| {
			panic!("unexpected event: {:?}", ev)
		});
		assert_eq!(result.unwrap(), true);
	}
}
