use rxml_proc::{xml_cdata, xml_name, xml_ncname};

const FOO: &'static rxml::CDataStr = xml_cdata!("foo");
const BAR: &'static rxml::NameStr = xml_name!("foo:bar");
const BAZ: &'static rxml::NCNameStr = xml_ncname!("foobar");

fn main() {
	todo!();
}
