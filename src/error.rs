use std::io;
use std::fmt;
use std::error;
use std::string;
use std::result::Result as StdResult;

#[derive(Debug)]
pub enum Error {
	IO(io::Error),
	Utf8(string::FromUtf8Error),
	InvalidStartByte(u8),
	InvalidContByte(u8),
	InvalidChar(u32),
	NotWellFormed(String),
	RestrictedXml(String),
}

pub type Result<T> = StdResult<T, Error>;

impl Error {
	pub fn io(e: io::Error) -> Error {
		Error::IO(e)
	}
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::io(e)
	}
}

impl From<string::FromUtf8Error> for Error {
	fn from(e: string::FromUtf8Error) -> Error {
		Error::Utf8(e)
	}
}

impl fmt::Display for Error {
	fn fmt<'f>(&self, f: &'f mut fmt::Formatter) -> fmt::Result {
		match self {
			Error::NotWellFormed(msg) => write!(f, "not well formed: {}", msg),
			Error::RestrictedXml(msg) => write!(f, "restricted xml: {}", msg),
			Error::InvalidStartByte(b) => write!(f, "invalid utf-8 start byte: \\x{:02x}", b),
			Error::InvalidContByte(b) => write!(f, "invalid utf-8 continuation byte: \\x{:02x}", b),
			Error::InvalidChar(ch) => write!(f, "invalid char: U+{:08x}", ch),
			Error::IO(e) => write!(f, "I/O error: {}", e),
			Error::Utf8(e) => write!(f, "utf8 error: {}", e),
		}
	}
}

impl error::Error for Error {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		match self {
			Error::IO(e) => Some(e),
			Error::Utf8(e) => Some(e),
			Error::NotWellFormed(_) |
				Error::RestrictedXml(_) |
				Error::InvalidStartByte(_) |
				Error::InvalidContByte(_) |
				Error::InvalidChar(_) => None,
		}
	}
}
