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

use std::fmt;
use std::ops::Add;

use bigint::BigUint;
use serde::{Serialize, Serializer, Deserialize, Deserializer, de};

use core::hash::Hash;
use ser::{self, Reader, Writer, Writeable, Readable};

/// The target is the 32-bytes hash block hashes must be lower than.
pub const MAX_TARGET: [u8; 32] = [0xf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                                  0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                                  0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];

/// The difficulty is defined as the maximum target divided by the block hash.
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct Difficulty {
	num: BigUint
}

impl Difficulty {
	/// Difficulty of one, which is the minumum difficulty (when the hash
	/// equals the
	/// max target)
	pub fn one() -> Difficulty {
		Difficulty { num: BigUint::new(vec![1]) }
	}

    /// Convert a `u32` into a `Difficulty`
	pub fn from_num(num: u32) -> Difficulty {
		Difficulty { num: BigUint::new(vec![num]) }
	}

    /// Convert a `BigUint` into a `Difficulty`
    pub fn from_biguint(num: BigUint) -> Difficulty {
        Difficulty { num: num }
    }

	/// Computes the difficulty from a hash. Divides the maximum target by the
	/// provided hash.
	pub fn from_hash(h: &Hash) -> Difficulty {
		let max_target = BigUint::from_bytes_be(&MAX_TARGET);
		let h_num = BigUint::from_bytes_be(&h[..]);
		Difficulty { num: max_target / h_num }
	}

    /// Converts the difficulty into a bignum
    pub fn into_biguint(self) -> BigUint {
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

impl Writeable for Difficulty {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		let data = self.num.to_bytes_be();
		try!(writer.write_u8(data.len() as u8));
		writer.write_fixed_bytes(&data)
	}
}

impl Readable for Difficulty {
	fn read(reader: &mut Reader) -> Result<Difficulty, ser::Error> {
		let dlen = try!(reader.read_u8());
		let data = try!(reader.read_fixed_bytes(dlen as usize));
		Ok(Difficulty { num: BigUint::from_bytes_be(&data[..]) })
	}
}

impl Serialize for Difficulty {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
		where S: Serializer
	{
		serializer.serialize_str(self.num.to_str_radix(10).as_str())
	}
}

impl Deserialize for Difficulty {
	fn deserialize<D>(deserializer: D) -> Result<Difficulty, D::Error>
		where D: Deserializer
	{
		deserializer.deserialize_i32(DiffVisitor)
	}
}

struct DiffVisitor;

impl de::Visitor for DiffVisitor {
	type Value = Difficulty;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("a difficulty")
	}

	fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
		where E: de::Error
	{
		let bigui = BigUint::parse_bytes(s.as_bytes(), 10).ok_or_else(|| {
        de::Error::invalid_value(de::Unexpected::Str(s), &"a value number")
      })?;
		Ok(Difficulty { num: bigui })
	}
}
