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

/// Types for a Cuckoo proof of work and its encapsulation as a fully usable
/// proof of work within a block header.
use std::cmp::max;
use std::ops::{Add, Div, Mul, Sub};
use std::{fmt, iter};

use rand::{thread_rng, Rng};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use consensus::SECOND_POW_SIZESHIFT;
use core::hash::Hashed;
use global;
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
		// can't have difficulty lower than 1
		Difficulty { num: max(num, 1) }
	}

	/// Computes the difficulty from a hash. Divides the maximum target by the
	/// provided hash and applies the Cuckoo sizeshift adjustment factor (see
	/// https://lists.launchpad.net/mimblewimble/msg00494.html).
	pub fn from_proof_adjusted(proof: &Proof) -> Difficulty {
		let max_target = <u64>::max_value();
		let target = proof.hash().to_u64();
		let shift = proof.cuckoo_sizeshift;

		// Adjust the difficulty based on a 2^(N-M)*(N-1) factor, with M being
		// the minimum sizeshift and N the provided sizeshift
		let adjust_factor = (1 << (shift - global::ref_sizeshift()) as u64) * (shift as u64 - 1);
		let difficulty = (max_target / target) * adjust_factor;

		Difficulty::from_num(difficulty)
	}

	/// Same as `from_proof_adjusted` but instead of an adjustment based on
	/// cycle size, scales based on a provided factor. Used by dual PoW system
	/// to scale one PoW against the other.
	pub fn from_proof_scaled(proof: &Proof, scaling: u64) -> Difficulty {
		let max_target = <u64>::max_value();
		let target = proof.hash().to_u64();

		// Scaling between 2 proof of work algos
		Difficulty::from_num((max_target / scaling) / target)
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

/// Block header information pertaining to the proof of work
#[derive(Clone, Debug, PartialEq)]
pub struct ProofOfWork {
	/// Total accumulated difficulty since genesis block
	pub total_difficulty: Difficulty,
	/// Difficulty scaling factor between the different proofs of work
	pub scaling_difficulty: u64,
	/// Nonce increment used to mine this block.
	pub nonce: u64,
	/// Proof of work data.
	pub proof: Proof,
}

impl Default for ProofOfWork {
	fn default() -> ProofOfWork {
		let proof_size = global::proofsize();
		ProofOfWork {
			total_difficulty: Difficulty::one(),
			scaling_difficulty: 1,
			nonce: 0,
			proof: Proof::zero(proof_size),
		}
	}
}

impl ProofOfWork {
	/// Read implementation, can't define as trait impl as we need a version
	pub fn read(ver: u16, reader: &mut Reader) -> Result<ProofOfWork, ser::Error> {
		let (total_difficulty, scaling_difficulty) = if ver == 1 {
			// read earlier in the header on older versions
			(Difficulty::one(), 1)
		} else {
			(Difficulty::read(reader)?, reader.read_u64()?)
		};
		let nonce = reader.read_u64()?;
		let proof = Proof::read(reader)?;
		Ok(ProofOfWork {
			total_difficulty,
			scaling_difficulty,
			nonce,
			proof,
		})
	}

	/// Write implementation, can't define as trait impl as we need a version
	pub fn write<W: Writer>(&self, ver: u16, writer: &mut W) -> Result<(), ser::Error> {
		if writer.serialization_mode() != ser::SerializationMode::Hash {
			self.write_pre_pow(ver, writer)?;
			writer.write_u64(self.nonce)?;
		}

		self.proof.write(writer)?;
		Ok(())
	}

	/// Write the pre-hash portion of the header
	pub fn write_pre_pow<W: Writer>(&self, ver: u16, writer: &mut W) -> Result<(), ser::Error> {
		if ver > 1 {
			ser_multiwrite!(
				writer,
				[write_u64, self.total_difficulty.to_num()],
				[write_u64, self.scaling_difficulty]
			);
		}
		Ok(())
	}

	/// Maximum difficulty this proof of work can achieve
	pub fn to_difficulty(&self) -> Difficulty {
		// 2 proof of works, Cuckoo29 (for now) and Cuckoo30+, which are scaled
		// differently (scaling not controlled for now)
		if self.proof.cuckoo_sizeshift == SECOND_POW_SIZESHIFT {
			Difficulty::from_proof_scaled(&self.proof, self.scaling_difficulty)
		} else {
			Difficulty::from_proof_adjusted(&self.proof)
		}
	}

	/// The shift used for the cuckoo cycle size on this proof
	pub fn cuckoo_sizeshift(&self) -> u8 {
		self.proof.cuckoo_sizeshift
	}
}

/// A Cuckoo Cycle proof of work, consisting of the shift to get the graph
/// size (i.e. 31 for Cuckoo31 with a 2^31 or 1<<31 graph size) and the nonces
/// of the graph solution. While being expressed as u64 for simplicity, each
/// nonce is strictly less than half the cycle size (i.e. <2^30 for Cuckoo 31).
///
/// The hash of the `Proof` is the hash of its packed nonces when serializing
/// them at their exact bit size. The resulting bit sequence is padded to be
/// byte-aligned.
///
#[derive(Clone, PartialOrd, PartialEq)]
pub struct Proof {
	/// Power of 2 used for the size of the cuckoo graph
	pub cuckoo_sizeshift: u8,
	/// The nonces
	pub nonces: Vec<u64>,
}

impl fmt::Debug for Proof {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Cuckoo{}(", self.cuckoo_sizeshift)?;
		for (i, val) in self.nonces[..].iter().enumerate() {
			write!(f, "{:x}", val)?;
			if i < self.nonces.len() - 1 {
				write!(f, " ")?;
			}
		}
		write!(f, ")")
	}
}

impl Eq for Proof {}

impl Proof {
	/// Builds a proof with provided nonces at default sizeshift
	pub fn new(mut in_nonces: Vec<u64>) -> Proof {
		in_nonces.sort();
		Proof {
			cuckoo_sizeshift: global::min_sizeshift(),
			nonces: in_nonces,
		}
	}

	/// Builds a proof with all bytes zeroed out
	pub fn zero(proof_size: usize) -> Proof {
		Proof {
			cuckoo_sizeshift: global::min_sizeshift(),
			nonces: vec![0; proof_size],
		}
	}

	/// Builds a proof with random POW data,
	/// needed so that tests that ignore POW
	/// don't fail due to duplicate hashes
	pub fn random(proof_size: usize) -> Proof {
		let sizeshift = global::min_sizeshift();
		let nonce_mask = (1 << (sizeshift - 1)) - 1;
		let mut rng = thread_rng();
		// force the random num to be within sizeshift bits
		let mut v: Vec<u64> = iter::repeat(())
			.map(|()| (rng.gen::<u32>() & nonce_mask) as u64)
			.take(proof_size)
			.collect();
		v.sort();
		Proof {
			cuckoo_sizeshift: global::min_sizeshift(),
			nonces: v,
		}
	}

	/// Returns the proof size
	pub fn proof_size(&self) -> usize {
		self.nonces.len()
	}
}

impl Readable for Proof {
	fn read(reader: &mut Reader) -> Result<Proof, ser::Error> {
		let cuckoo_sizeshift = reader.read_u8()?;
		if cuckoo_sizeshift == 0 || cuckoo_sizeshift > 64 {
			return Err(ser::Error::CorruptedData);
		}

		let mut nonces = Vec::with_capacity(global::proofsize());
		let nonce_bits = cuckoo_sizeshift as usize - 1;
		let bytes_len = BitVec::bytes_len(nonce_bits * global::proofsize());
		let bits = reader.read_fixed_bytes(bytes_len)?;
		let bitvec = BitVec { bits };
		for n in 0..global::proofsize() {
			let mut nonce = 0;
			for bit in 0..nonce_bits {
				if bitvec.bit_at(n * nonce_bits + (bit as usize)) {
					nonce |= 1 << bit;
				}
			}
			nonces.push(nonce);
		}
		Ok(Proof {
			cuckoo_sizeshift,
			nonces,
		})
	}
}

impl Writeable for Proof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		if writer.serialization_mode() != ser::SerializationMode::Hash {
			writer.write_u8(self.cuckoo_sizeshift)?;
		}

		let nonce_bits = self.cuckoo_sizeshift as usize - 1;
		let mut bitvec = BitVec::new(nonce_bits * global::proofsize());
		for (n, nonce) in self.nonces.iter().enumerate() {
			for bit in 0..nonce_bits {
				if nonce & (1 << bit) != 0 {
					bitvec.set_bit_at(n * nonce_bits + (bit as usize))
				}
			}
		}
		writer.write_fixed_bytes(&bitvec.bits)?;
		Ok(())
	}
}

// TODO this could likely be optimized by writing whole bytes (or even words)
// in the `BitVec` at once, dealing with the truncation, instead of bits by bits
struct BitVec {
	bits: Vec<u8>,
}

impl BitVec {
	/// Number of bytes required to store the provided number of bits
	fn bytes_len(bits_len: usize) -> usize {
		(bits_len + 7) / 8
	}

	fn new(bits_len: usize) -> BitVec {
		BitVec {
			bits: vec![0; BitVec::bytes_len(bits_len)],
		}
	}

	fn set_bit_at(&mut self, pos: usize) {
		self.bits[pos / 8] |= 1 << (pos % 8) as u8;
	}

	fn bit_at(&self, pos: usize) -> bool {
		self.bits[pos / 8] & (1 << (pos % 8) as u8) != 0
	}
}
