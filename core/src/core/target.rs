// Copyright 2018 The Grin Developers
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
use std::cmp::max;

use core::global;
use core::hash::Hash;
use ser::{self, Readable, Reader, Writeable, Writer};

/// The difficulty is defined as the maximum target divided by the block hash.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
pub struct Difficulty {
	num: u64,
}

impl Difficulty {
	/// Difficulty of zero, which is invalid (no target can be
	/// calculated from it) but very useful as a start for additions.
	pub fn zero() -> Difficulty {
		Difficulty { num: 0 }
	}

	/// Difficulty of one, which is the minimum difficulty
	/// (when the hash equals the max target)
	pub fn one() -> Difficulty {
		Difficulty { num: 1 }
	}

	/// Convert a `u32` into a `Difficulty`
	pub fn from_num(num: u64) -> Difficulty {
		Difficulty { num: num }
	}

	/// Computes the difficulty from a hash. Divides the maximum target by the
	/// provided hash and applies the Cuckoo sizeshift adjustment factor (see
	/// https://lists.launchpad.net/mimblewimble/msg00494.html).
	pub fn from_hash_and_shift(h: &Hash, shift: u8) -> Difficulty {
		let max_target = <u64>::max_value();
		let num = h.to_u64();
		// Adjust the difficulty based on a 2^(N-M)*(N-1) factor, with M being
		// the minimum sizeshift and N the provided sizeshift
		let adjust_factor = (1 << (shift - global::ref_sizeshift()) as u64) * (shift as u64 - 1);
		Difficulty {
			num: (max_target / max(num, adjust_factor)) * adjust_factor,
		}
	}

	/// Converts the difficulty into a u64
	pub fn to_num(&self) -> u64 {
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
		Difficulty {
			num: self.num + other.num,
		}
	}
}

impl Sub<Difficulty> for Difficulty {
	type Output = Difficulty;
	fn sub(self, other: Difficulty) -> Difficulty {
		Difficulty {
			num: self.num - other.num,
		}
	}
}

impl Mul<Difficulty> for Difficulty {
	type Output = Difficulty;
	fn mul(self, other: Difficulty) -> Difficulty {
		Difficulty {
			num: self.num * other.num,
		}
	}
}

impl Div<Difficulty> for Difficulty {
	type Output = Difficulty;
	fn div(self, other: Difficulty) -> Difficulty {
		Difficulty {
			num: self.num / other.num,
		}
	}
}

impl Writeable for Difficulty {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.num)
	}
}

impl Readable for Difficulty {
	fn read(reader: &mut Reader) -> Result<Difficulty, ser::Error> {
		let data = reader.read_u64()?;
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
		if num_in.is_err() {
			return Err(de::Error::invalid_value(
				de::Unexpected::Str(s),
				&"a value number",
			));
		};
		Ok(Difficulty {
			num: num_in.unwrap(),
		})
	}

	fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
	where
		E: de::Error,
	{
		Ok(Difficulty { num: value })
	}
}
