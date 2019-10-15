use std::ops::Deref;
use zeroize::Zeroize;
/// Zeroing string, mainly useful for password
#[derive(Clone, PartialEq, PartialOrd)]
pub struct ZeroingString(String);

impl Drop for ZeroingString {
	fn drop(&mut self) {
		self.0.zeroize();
	}
}

impl From<&str> for ZeroingString {
	fn from(s: &str) -> Self {
		ZeroingString(String::from(s))
	}
}

impl From<String> for ZeroingString {
	fn from(s: String) -> Self {
		ZeroingString(s)
	}
}

impl Deref for ZeroingString {
	type Target = str;

	fn deref(&self) -> &str {
		&self.0
	}
}
