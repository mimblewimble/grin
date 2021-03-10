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

//! Zeroing String

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
