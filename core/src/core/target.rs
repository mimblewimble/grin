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

//! Definition of the maximum target value a proof-of-work block hash can have
//! and
//! the related difficulty, defined as the maximum target divided by the hash.
//!
//! Note this is now wrapping a simple U64 now, but it's desirable to keep the
//! wrapper in case the internal representation needs to change again

use std::fmt;
use std::ops::{Add, Div, Mul, Sub};

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use byteorder::{BigEndian, ByteOrder};

use core::hash::Hash;
use ser::{self, Readable, Reader, Writeable, Writer};

/// The target is the 32-bytes hash block hashes must be lower than.
pub const MAX_TARGET: [u8; 8] = [0xf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];

/// The difficulty is defined as the maximum target divided by the block hash.
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct Difficulty {
	num: u64,
}

impl Difficulty {
	/// Difficulty of zero, which is practically invalid (not target can be
	/// calculated from it) but very useful as a start for additions.
	pub fn zero() -> Difficulty {
		Difficulty { num: 0 }
	}

	/// Difficulty of one, which is the minumum difficulty (when the hash
	/// equals the max target)
	pub fn one() -> Difficulty {
		Difficulty { num: 1 }
	}

	/// Convert a `u32` into a `Difficulty`
	pub fn from_num(num: u64) -> Difficulty {
		Difficulty { num: num }
	}

	/// Computes the difficulty from a hash. Divides the maximum target by the
	/// provided hash.
	pub fn from_hash(h: &Hash) -> Difficulty {
		let max_target = BigEndian::read_u64(&MAX_TARGET);
		// Use the first 64 bits of the given hash
		let mut in_vec = h.to_vec();
		in_vec.truncate(8);
		let num = BigEndian::read_u64(&in_vec);
		Difficulty { num: max_target / num }
	}

	/// Converts the difficulty into a u64
	pub fn into_num(&self) -> u64 {
		self.num
	}
}

impl fmt::Display for Difficulty {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}", self.num)
	}
}

impl Add<Difficulty> for Difficulty {
	type Output = Difficulty;
	fn add(self, other: Difficulty) -> Difficulty {
		Difficulty { num: self.num + other.num }
	}
}

impl Sub<Difficulty> for Difficulty {
	type Output = Difficulty;
	fn sub(self, other: Difficulty) -> Difficulty {
		Difficulty { num: self.num - other.num }
	}
}

impl Mul<Difficulty> for Difficulty {
	type Output = Difficulty;
	fn mul(self, other: Difficulty) -> Difficulty {
		Difficulty { num: self.num * other.num }
	}
}

impl Div<Difficulty> for Difficulty {
	type Output = Difficulty;
	fn div(self, other: Difficulty) -> Difficulty {
		Difficulty { num: self.num / other.num }
	}
}

impl Writeable for Difficulty {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.num)
	}
}

impl Readable for Difficulty {
	fn read(reader: &mut Reader) -> Result<Difficulty, ser::Error> {
		let data = try!(reader.read_u64());
		Ok(Difficulty { num: data })
	}
}

impl Serialize for Difficulty {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_u64(self.num)
	}
}

impl<'de> Deserialize<'de> for Difficulty {
	fn deserialize<D>(deserializer: D) -> Result<Difficulty, D::Error>
	where
		D: Deserializer<'de>,
	{
		deserializer.deserialize_u64(DiffVisitor)
	}
}

struct DiffVisitor;

impl<'de> de::Visitor<'de> for DiffVisitor {
	type Value = Difficulty;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("a difficulty")
	}

	fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
	where
		E: de::Error,
	{
		let num_in = s.parse::<u64>();
		if let Err(_) = num_in {
			return Err(de::Error::invalid_value(
				de::Unexpected::Str(s),
				&"a value number",
			));
		};
		Ok(Difficulty { num: num_in.unwrap() })
	}
}
