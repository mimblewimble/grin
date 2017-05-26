// Copyright 2016 The Grin Developers
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
use std::num;

/// Encode the provided bytes into a hex string
pub fn to_hex(bytes: Vec<u8>) -> String {
  let mut s = String::new();
  for byte in bytes {
    write!(&mut s, "{:02X}", byte).expect("Unable to write");
  }
  s
}

/// Decode a hex string into bytes.
pub fn from_hex(hex_str: String) -> Result<Vec<u8>, num::ParseIntError> {
  let hex_trim = if &hex_str[..2] == "0x" {
    hex_str[2..].to_owned()
  } else {
    hex_str.clone()
  };
  split_n(&hex_trim.trim()[..], 2).iter()
    .map(|b| u8::from_str_radix(b, 16))
    .collect::<Result<Vec<u8>, _>>()
}

fn split_n(s: &str, n: usize) -> Vec<&str> {
  (0 .. (s.len() - n + 1)/2 + 1).map(|i| &s[2*i .. 2*i + n]).collect()
}
