use rxml_proc::{xml_cdata, xml_name, xml_ncname};
use rxml_validation::{CDataStr, NCNameStr, NameStr};

const FOO: &'static CDataStr = xml_cdata!("foo");
const BAR: &'static NameStr = xml_name!("foo:bar");
const BAZ: &'static NCNameStr = xml_ncname!("foobar");

fn main() {
	println!("{:?} {:?} {:?}", FOO, BAR, BAZ);
}
