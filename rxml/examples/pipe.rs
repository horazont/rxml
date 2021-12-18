use std::io;
use std::io::Write;

use bytes::BytesMut;

use rxml::writer::Encoder;
use rxml::{Error, EventRead, PullParser};

fn main() {
	let stdin = io::BufReader::new(io::stdin());
	let mut stdout = io::stdout();
	let mut enc = Encoder::new();
	let mut parser = PullParser::new(stdin);
	let mut buf = BytesMut::with_capacity(8192);
	let result = parser.read_all(|ev| {
		enc.encode_event_into_bytes(&ev, &mut buf)
			.expect("failed to encode xml");
		stdout
			.write_all(&buf[..])
			.expect("failed to write to stdout");
		buf.clear();
	});
	match result {
		Ok(()) => (),
		Err(Error::IO(e)) => panic!("I/O error: {}", e),
		Err(e) => panic!("invalid XML on input: {}", e),
	}
}
