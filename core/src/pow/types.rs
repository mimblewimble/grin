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

/// Types for a Cuck(at)oo proof of work and its encapsulation as a fully usable
/// proof of work within a block header.
use std::cmp::max;
use std::ops::{Add, Div, Mul, Sub};
use std::{fmt, iter};

use rand::{thread_rng, Rng};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use consensus::SECOND_POW_EDGE_BITS;
use core::hash::Hashed;
use global;
use ser::{self, Readable, Reader, Writeable, Writer};

use pow::common::EdgeType;
use pow::error::Error;

/// Generic trait for a solver/verifier providing common interface into Cuckoo-family PoW
/// Mostly used for verification, but also for test mining if necessary
pub trait PoWContext<T>
where
	T: EdgeType,
{
	/// Create new instance of context with appropriate parameters
	fn new(
		edge_bits: u8,
		proof_size: usize,
		max_sols: u32,
	) -> Result<Box<Self>, Error>;
	/// Sets the header along with an optional nonce at the end
	/// solve: whether to set up structures for a solve (true) or just validate (false)
	fn set_header_nonce(
		&mut self,
		header: Vec<u8>,
		nonce: Option<u32>,
		solve: bool,
	) -> Result<(), Error>;
	/// find solutions using the stored parameters and header
	fn find_cycles(&mut self) -> Result<Vec<Proof>, Error>;
	/// Verify a solution with the stored parameters
	fn verify(&self, proof: &Proof) -> Result<(), Error>;
}

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

	/// Compute difficulty scaling factor for graph defined by 2 * 2^edge_bits * edge_bits bits
	pub fn scale(edge_bits: u8) -> u64 {
		(2 << (edge_bits - global::base_edge_bits()) as u64) * (edge_bits as u64)
	}

	/// Computes the difficulty from a hash. Divides the maximum target by the
	/// provided hash and applies the Cuck(at)oo size adjustment factor (see
	/// https://lists.launchpad.net/mimblewimble/msg00494.html).
	fn from_proof_adjusted(proof: &Proof) -> Difficulty {
		// Adjust the difficulty based on a 2^(N-M)*(N-1) factor, with M being
		// the minimum edge_bits and N the provided edge_bits
		let edge_bits = proof.edge_bits;

		Difficulty::from_num(proof.raw_difficulty() * Difficulty::scale(edge_bits))
	}

	/// Same as `from_proof_adjusted` but instead of an adjustment based on
	/// cycle size, scales based on a provided factor. Used by dual PoW system
	/// to scale one PoW against the other.
	fn from_proof_scaled(proof: &Proof, scaling: u32) -> Difficulty {
		// Scaling between 2 proof of work algos
		Difficulty::from_num(proof.raw_difficulty() * scaling as u64)
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
	pub scaling_difficulty: u32,
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
	pub fn read(_ver: u16, reader: &mut Reader) -> Result<ProofOfWork, ser::Error> {
		let total_difficulty = Difficulty::read(reader)?;
		let scaling_difficulty = reader.read_u32()?;
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
	pub fn write_pre_pow<W: Writer>(&self, _ver: u16, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u64, self.total_difficulty.to_num()],
			[write_u32, self.scaling_difficulty]
		);
		Ok(())
	}

	/// Maximum difficulty this proof of work can achieve
	pub fn to_difficulty(&self) -> Difficulty {
		// 2 proof of works, Cuckoo29 (for now) and Cuckoo30+, which are scaled
		// differently (scaling not controlled for now)
		if self.proof.edge_bits == SECOND_POW_EDGE_BITS {
			Difficulty::from_proof_scaled(&self.proof, self.scaling_difficulty)
		} else {
			Difficulty::from_proof_adjusted(&self.proof)
		}
	}

	/// The edge_bits used for the cuckoo cycle size on this proof
	pub fn edge_bits(&self) -> u8 {
		self.proof.edge_bits
	}

	/// Whether this proof of work is for the primary algorithm (as opposed
	/// to secondary). Only depends on the edge_bits at this time.
	pub fn is_primary(&self) -> bool {
		// 2 conditions are redundant right now but not necessarily in
		// the future
		self.proof.edge_bits != SECOND_POW_EDGE_BITS
			&& self.proof.edge_bits >= global::min_edge_bits()
	}

	/// Whether this proof of work is for the secondary algorithm (as opposed
	/// to primary). Only depends on the edge_bits at this time.
	pub fn is_secondary(&self) -> bool {
		self.proof.edge_bits == SECOND_POW_EDGE_BITS
	}
}

/// A Cuck(at)oo Cycle proof of work, consisting of the edge_bits to get the graph
/// size (i.e. the 2-log of the number of edges) and the nonces
/// of the graph solution. While being expressed as u64 for simplicity,
/// nonces a.k.a. edge indices range from 0 to (1 << edge_bits) - 1
///
/// The hash of the `Proof` is the hash of its packed nonces when serializing
/// them at their exact bit size. The resulting bit sequence is padded to be
/// byte-aligned.
///
#[derive(Clone, PartialOrd, PartialEq)]
pub struct Proof {
	/// Power of 2 used for the size of the cuckoo graph
	pub edge_bits: u8,
	/// The nonces
	pub nonces: Vec<u64>,
}

impl fmt::Debug for Proof {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Cuckoo{}(", self.edge_bits)?;
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
	/// Builds a proof with provided nonces at default edge_bits
	pub fn new(mut in_nonces: Vec<u64>) -> Proof {
		in_nonces.sort();
		Proof {
			edge_bits: global::min_edge_bits(),
			nonces: in_nonces,
		}
	}

	/// Builds a proof with all bytes zeroed out
	pub fn zero(proof_size: usize) -> Proof {
		Proof {
			edge_bits: global::min_edge_bits(),
			nonces: vec![0; proof_size],
		}
	}

	/// Builds a proof with random POW data,
	/// needed so that tests that ignore POW
	/// don't fail due to duplicate hashes
	pub fn random(proof_size: usize) -> Proof {
		let edge_bits = global::min_edge_bits();
		let nonce_mask = (1 << edge_bits) - 1;
		let mut rng = thread_rng();
		// force the random num to be within edge_bits bits
		let mut v: Vec<u64> = iter::repeat(())
			.map(|()| (rng.gen::<u32>() & nonce_mask) as u64)
			.take(proof_size)
			.collect();
		v.sort();
		Proof {
			edge_bits: global::min_edge_bits(),
			nonces: v,
		}
	}

	/// Returns the proof size
	pub fn proof_size(&self) -> usize {
		self.nonces.len()
	}

	/// Difficulty achieved by this proof
	fn raw_difficulty(&self) -> u64 {
		<u64>::max_value() / self.hash().to_u64()
	}
}

impl Readable for Proof {
	fn read(reader: &mut Reader) -> Result<Proof, ser::Error> {
		let edge_bits = reader.read_u8()?;
		if edge_bits == 0 || edge_bits > 64 {
			return Err(ser::Error::CorruptedData);
		}

		let mut nonces = Vec::with_capacity(global::proofsize());
		let nonce_bits = edge_bits as usize;
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
			edge_bits,
			nonces,
		})
	}
}

impl Writeable for Proof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		if writer.serialization_mode() != ser::SerializationMode::Hash {
			writer.write_u8(self.edge_bits)?;
		}
		let nonce_bits = self.edge_bits as usize;
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
