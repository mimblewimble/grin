// Copyright 2021 The Grin Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/// Implements hex-encoding from bytes to string and decoding of strings
/// to bytes. Given that rustc-serialize is deprecated and serde doesn't
/// provide easy hex encoding, hex is a bit in limbo right now in Rust-
/// land. It's simple enough that we can just have our own.
use std::fmt::Write;

/// Encode the provided bytes into a hex string
fn to_hex(bytes: &[u8]) -> String {
	let mut s = String::with_capacity(bytes.len() * 2);
	for byte in bytes {
		write!(&mut s, "{:02x}", byte).expect("Unable to write hex");
	}
	s
}

/// Convert to hex
pub trait ToHex {
	/// convert to hex
	fn to_hex(&self) -> String;
}

impl<T: AsRef<[u8]>> ToHex for T {
	fn to_hex(&self) -> String {
		to_hex(self.as_ref())
	}
}

/// Decode a hex string into bytes.
pub fn from_hex(hex: &str) -> Result<Vec<u8>, String> {
	let hex = hex.trim().trim_start_matches("0x");
	if hex.len() % 2 != 0 {
		Err(hex.to_string())
	} else {
		(0..hex.len())
			.step_by(2)
			.map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| hex.to_string()))
			.collect()
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn test_to_hex() {
		assert_eq!(vec![0, 0, 0, 0].to_hex(), "00000000");
		assert_eq!(vec![10, 11, 12, 13].to_hex(), "0a0b0c0d");
		assert_eq!([0, 0, 0, 255].to_hex(), "000000ff");
	}

	#[test]
	fn test_to_hex_trait() {
		assert_eq!(vec![0, 0, 0, 0].to_hex(), "00000000");
		assert_eq!(vec![10, 11, 12, 13].to_hex(), "0a0b0c0d");
		assert_eq!([0, 0, 0, 255].to_hex(), "000000ff");
	}

	#[test]
	fn test_from_hex() {
		assert_eq!(from_hex(""), Ok(vec![]));
		assert_eq!(from_hex("00000000"), Ok(vec![0, 0, 0, 0]));
		assert_eq!(from_hex("0a0b0c0d"), Ok(vec![10, 11, 12, 13]));
		assert_eq!(from_hex("000000ff"), Ok(vec![0, 0, 0, 255]));
		assert_eq!(from_hex("0x000000ff"), Ok(vec![0, 0, 0, 255]));
		assert_eq!(from_hex("0x000000fF"), Ok(vec![0, 0, 0, 255]));
		assert_eq!(from_hex("0x000000fg"), Err("000000fg".to_string()));
		assert_eq!(
			from_hex("not a hex string"),
			Err("not a hex string".to_string())
		);
		assert_eq!(from_hex("0"), Err("0".to_string()));
	}
}
