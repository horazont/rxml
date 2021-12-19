use super::*;

use std::convert::TryFrom;

#[cfg(feature = "async")]
use tokio;

// note that this is just a smoketest... the components of the FeedParser
// are tested extensively in the modules.
#[test]
fn feedparser_can_read_xml_document() {
	let doc = b"<?xml version='1.0'?>\n<root xmlns='urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4' a=\"foo\" b='bar'><child>with some text</child></root>";

	let mut fp = FeedParser::new();
	let mut out = Vec::<Event>::new();
	fp.feed(&doc[..]);
	let result = fp.read_all_eof(|ev| {
		out.push(ev);
	});
	assert_eq!(result.unwrap(), false);

	{
		let mut iter = out.iter();
		match iter.next().unwrap() {
			Event::XMLDeclaration(em, XMLVersion::V1_0) => {
				assert_eq!(em.len(), 21);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				// note: 77 because of the \n between xml decl and whitespace. see also comment on EventMetrics
				assert_eq!(em.len(), 77);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "root");
				assert_eq!(attrs.len(), 2);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("a").unwrap())).unwrap(),
					"foo"
				);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("b").unwrap())).unwrap(),
					"bar"
				);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len(), 7);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "child");
				assert_eq!(attrs.len(), 0);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::Text(em, cdata) => {
				assert_eq!(em.len(), 14);
				assert_eq!(cdata, "with some text");
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 8);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 7);
			}
			other => panic!("unexpected event: {:?}", other),
		};
	}

	fp.feed_eof();
	let result = fp.read_all_eof(|ev| panic!("unexpected event: {:?}", ev));
	assert_eq!(result.unwrap(), true);
}

#[test]
fn feedparser_can_handle_chunked_input() {
	let doc = b"<?xml version='1.0'?><root xmlns='urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4' a=\"foo\" b='bar'><child>with some text</child></root>";

	let mut fp = FeedParser::new();
	let mut out = Vec::<Event>::new();
	for chunk in doc.chunks(10) {
		fp.feed(chunk.to_vec());
		loop {
			match fp.read() {
				Err(Error::IO(ioerr)) if ioerr.kind() == io::ErrorKind::WouldBlock => break,
				Err(other) => panic!("unexpected error: {:?}", other),
				Ok(Some(ev)) => out.push(ev),
				Ok(None) => break,
			}
		}
	}

	{
		let mut iter = out.iter();
		match iter.next().unwrap() {
			Event::XMLDeclaration(em, XMLVersion::V1_0) => {
				assert_eq!(em.len(), 21);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len(), 76);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "root");
				assert_eq!(attrs.len(), 2);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("a").unwrap())).unwrap(),
					"foo"
				);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("b").unwrap())).unwrap(),
					"bar"
				);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len(), 7);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "child");
				assert_eq!(attrs.len(), 0);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::Text(em, cdata) => {
				assert_eq!(em.len(), 14);
				assert_eq!(cdata, "with some text");
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 8);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 7);
			}
			other => panic!("unexpected event: {:?}", other),
		};
	}

	fp.feed_eof();
	let result = fp.read_all_eof(|ev| panic!("unexpected event: {:?}", ev));
	assert_eq!(result.unwrap(), true);
}

// note that this is just a smoketest... the components of the PullParser
// are tested extensively in the modules.
#[test]
fn pullparser_can_read_xml_document() {
	let mut doc = &b"<?xml version='1.0'?>\n<root xmlns='urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4' a=\"foo\" b='bar'><child>with some text</child></root>\n"[..];

	let mut pp = PullParser::new(&mut doc);
	let mut out = Vec::<Event>::new();
	let result = pp.read_all(|ev| {
		out.push(ev);
	});
	assert_eq!(result.unwrap(), ());

	{
		let mut iter = out.iter();
		match iter.next().unwrap() {
			Event::XMLDeclaration(em, XMLVersion::V1_0) => {
				assert_eq!(em.len(), 21);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				// note: 77 because of the \n between xml decl and whitespace. see also comment on EventMetrics
				assert_eq!(em.len(), 77);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "root");
				assert_eq!(attrs.len(), 2);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("a").unwrap())).unwrap(),
					"foo"
				);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("b").unwrap())).unwrap(),
					"bar"
				);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len(), 7);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "child");
				assert_eq!(attrs.len(), 0);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::Text(em, cdata) => {
				assert_eq!(em.len(), 14);
				assert_eq!(cdata, "with some text");
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 8);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 7);
			}
			other => panic!("unexpected event: {:?}", other),
		};
	}
}

/// This is only used to drop-in tests with util/fuzz-to-test.py
#[allow(dead_code)]
fn run_fuzz_test(data: &[u8]) -> Result<()> {
	let mut fp = FeedParser::new();
	fp.feed(data);
	fp.feed_eof();
	loop {
		match fp.read() {
			Ok(None) => return Ok(()),
			Err(e) => return Err(e),
			Ok(Some(_)) => (),
		}
	}
}

#[cfg(feature = "async")]
#[tokio::test]
async fn asyncparser_can_read_xml_document() {
	let doc = b"<?xml version='1.0'?>\n<root xmlns='urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4' a=\"foo\" b='bar'><child>with some text</child></root>";

	let mut r = &doc[..];
	let mut ap = AsyncParser::new(&mut r);
	let mut out = Vec::<Event>::new();
	let result = ap
		.read_all(|ev| {
			out.push(ev);
		})
		.await;
	result.unwrap();

	{
		let mut iter = out.iter();
		match iter.next().unwrap() {
			Event::XMLDeclaration(em, XMLVersion::V1_0) => {
				assert_eq!(em.len(), 21);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				// note: 77 because of the \n between xml decl and whitespace. see also comment on EventMetrics
				assert_eq!(em.len(), 77);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "root");
				assert_eq!(attrs.len(), 2);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("a").unwrap())).unwrap(),
					"foo"
				);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("b").unwrap())).unwrap(),
					"bar"
				);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len(), 7);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "child");
				assert_eq!(attrs.len(), 0);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::Text(em, cdata) => {
				assert_eq!(em.len(), 14);
				assert_eq!(cdata, "with some text");
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 8);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 7);
			}
			other => panic!("unexpected event: {:?}", other),
		};
	}
}

#[cfg(feature = "async")]
#[tokio::test]
async fn asyncparser_can_handle_chunked_input() {
	let doc = b"<?xml version='1.0'?>\n<root xmlns='urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4' a=\"foo\" b='bar'><child>with some text</child></root>";

	let mut r = &doc[..];
	let mut r = tokio::io::BufReader::with_capacity(4, &mut r);
	let mut ap = AsyncParser::new(&mut r);
	let mut out = Vec::<Event>::new();
	let result = ap
		.read_all(|ev| {
			out.push(ev);
		})
		.await;
	result.unwrap();

	{
		let mut iter = out.iter();
		match iter.next().unwrap() {
			Event::XMLDeclaration(em, XMLVersion::V1_0) => {
				assert_eq!(em.len(), 21);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				// note: 77 because of the \n between xml decl and whitespace. see also comment on EventMetrics
				assert_eq!(em.len(), 77);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "root");
				assert_eq!(attrs.len(), 2);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("a").unwrap())).unwrap(),
					"foo"
				);
				assert_eq!(
					attrs.get(&(None, NCName::try_from("b").unwrap())).unwrap(),
					"bar"
				);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::StartElement(em, (nsuri, localname), attrs) => {
				assert_eq!(em.len(), 7);
				assert_eq!(
					nsuri.as_ref().unwrap().as_str(),
					"urn:uuid:fab98e86-7c09-477c-889c-0313d9877bb4"
				);
				assert_eq!(localname, "child");
				assert_eq!(attrs.len(), 0);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::Text(em, cdata) => {
				assert_eq!(em.len(), 14);
				assert_eq!(cdata, "with some text");
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 8);
			}
			other => panic!("unexpected event: {:?}", other),
		};
		match iter.next().unwrap() {
			Event::EndElement(em) => {
				assert_eq!(em.len(), 7);
			}
			other => panic!("unexpected event: {:?}", other),
		};
	}
}
