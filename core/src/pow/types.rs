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

use crate::consensus::{graph_weight, MIN_DMA_DIFFICULTY, SECOND_POW_EDGE_BITS};
use crate::core::hash::{DefaultHashable, Hashed};
use crate::global;
use crate::pow::error::Error;
use crate::ser::{self, DeserializationMode, Readable, Reader, Writeable, Writer};
use rand::{thread_rng, Rng};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
/// Types for a Cuck(at)oo proof of work and its encapsulation as a fully usable
/// proof of work within a block header.
use std::cmp::{max, min};
use std::ops::{Add, Div, Mul, Sub};
use std::u64;
use std::{fmt, iter};

/// Generic trait for a solver/verifier providing common interface into Cuckoo-family PoW
/// Mostly used for verification, but also for test mining if necessary
pub trait PoWContext {
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

	/// Difficulty of MIN_DIFFICULTY
	pub fn min_dma() -> Difficulty {
		Difficulty {
			num: MIN_DMA_DIFFICULTY,
		}
	}

	/// Difficulty of MIN_DIFFICULTY
	pub fn min_wtema() -> Difficulty {
		Difficulty {
			num: global::min_wtema_graph_weight(),
		}
	}

	/// Difficulty unit, which is the graph weight of minimal graph
	pub fn unit() -> Difficulty {
		Difficulty {
			num: global::initial_graph_weight() as u64,
		}
	}

	/// Convert a `u32` into a `Difficulty`
	pub fn from_num(num: u64) -> Difficulty {
		// can't have difficulty lower than 1
		Difficulty { num: max(num, 1) }
	}

	/// Computes the difficulty from a hash. Divides the maximum target by the
	/// provided hash and applies the Cuck(at)oo size adjustment factor (see
	/// https://lists.launchpad.net/mimblewimble/msg00494.html).
	fn from_proof_adjusted(height: u64, proof: &Proof) -> Difficulty {
		// scale with natural scaling factor
		Difficulty::from_num(proof.scaled_difficulty(graph_weight(height, proof.edge_bits)))
	}

	/// Same as `from_proof_adjusted` but instead of an adjustment based on
	/// cycle size, scales based on a provided factor. Used by dual PoW system
	/// to scale one PoW against the other.
	fn from_proof_scaled(proof: &Proof, scaling: u32) -> Difficulty {
		// Scaling between 2 proof of work algos
		Difficulty::from_num(proof.scaled_difficulty(scaling as u64))
	}

	/// Converts the difficulty into a u64
	pub fn to_num(self) -> u64 {
		self.num
	}
}

impl fmt::Display for Difficulty {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
	fn read<R: Reader>(reader: &mut R) -> Result<Difficulty, ser::Error> {
		let data = reader.read_u64()?;
		Ok(Difficulty { num: data })
	}
}

impl Difficulty {
	/// Difficulty is 8 bytes.
	pub const LEN: usize = 8;
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

	fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProofOfWork {
	/// Total accumulated difficulty since genesis block
	pub total_difficulty: Difficulty,
	/// Variable difficulty scaling factor fo secondary proof of work
	/// After HardFork4, which obsoletes secondary PoW, this effectively
	/// becomes 4 more bytes of nonce. might be repurposed in future.
	pub secondary_scaling: u32,
	/// Nonce increment used to mine this block.
	pub nonce: u64,
	/// Proof of work data.
	pub proof: Proof,
}

impl Default for ProofOfWork {
	fn default() -> ProofOfWork {
		let proof_size = global::proofsize();
		ProofOfWork {
			total_difficulty: Difficulty::min_dma(),
			secondary_scaling: 1,
			nonce: 0,
			proof: Proof::zero(proof_size),
		}
	}
}

impl Writeable for ProofOfWork {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		if writer.serialization_mode() != ser::SerializationMode::Hash {
			self.write_pre_pow(writer)?;
			writer.write_u64(self.nonce)?;
		}
		self.proof.write(writer)?;
		Ok(())
	}
}

impl Readable for ProofOfWork {
	fn read<R: Reader>(reader: &mut R) -> Result<ProofOfWork, ser::Error> {
		let total_difficulty = Difficulty::read(reader)?;
		let secondary_scaling = reader.read_u32()?;
		let nonce = reader.read_u64()?;
		let proof = Proof::read(reader)?;
		Ok(ProofOfWork {
			total_difficulty,
			secondary_scaling,
			nonce,
			proof,
		})
	}
}

impl ProofOfWork {
	/// Write the pre-hash portion of the header
	pub fn write_pre_pow<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u64, self.total_difficulty.to_num()],
			[write_u32, self.secondary_scaling]
		);
		Ok(())
	}

	/// Maximum difficulty this proof of work can achieve
	pub fn to_difficulty(&self, height: u64) -> Difficulty {
		// 2 proof of works, Cuckoo29 (for now) and Cuckoo30+, which are scaled
		// differently (scaling not controlled for now)
		if self.proof.edge_bits == SECOND_POW_EDGE_BITS {
			Difficulty::from_proof_scaled(&self.proof, self.secondary_scaling)
		} else {
			Difficulty::from_proof_adjusted(height, &self.proof)
		}
	}

	/// Maximum unscaled difficulty this proof of work can achieve
	pub fn to_unscaled_difficulty(&self) -> Difficulty {
		// using scale = 1 gives "unscaled" value
		Difficulty::from_num(self.proof.scaled_difficulty(1u64))
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
/// byte-aligned. We form a PROOFSIZE*edge_bits integer by packing the PROOFSIZE edge
/// indices together, with edge index i occupying bits i * edge_bits through
/// (i+1) * edge_bits - 1, padding it with up to 7 0-bits to a multiple of 8 bits,
/// writing as a little endian byte array, and hashing with blake2b using 256 bit digest.

#[derive(Clone, PartialOrd, PartialEq, Serialize)]
pub struct Proof {
	/// Power of 2 used for the size of the cuckoo graph
	pub edge_bits: u8,
	/// The nonces
	pub nonces: Vec<u64>,
}

impl DefaultHashable for Proof {}

impl fmt::Debug for Proof {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
		in_nonces.sort_unstable();
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

	/// Number of bytes required store a proof of given edge bits
	pub fn pack_len(bit_width: u8) -> usize {
		(bit_width as usize * global::proofsize() + 7) / 8
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
		v.sort_unstable();
		Proof {
			edge_bits: global::min_edge_bits(),
			nonces: v,
		}
	}

	/// Returns the proof size
	pub fn proof_size(&self) -> usize {
		self.nonces.len()
	}

	/// Pack the nonces of the proof to their exact bit size as described above
	pub fn pack_nonces(&self) -> Vec<u8> {
		let mut compressed = vec![0u8; Proof::pack_len(self.edge_bits)];
		pack_bits(
			self.edge_bits,
			&self.nonces[0..self.nonces.len()],
			&mut compressed,
		);
		compressed
	}

	/// Difficulty achieved by this proof with given scaling factor
	fn scaled_difficulty(&self, scale: u64) -> u64 {
		let diff = ((scale as u128) << 64) / (max(1, self.hash().to_u64()) as u128);
		min(diff, <u64>::max_value() as u128) as u64
	}
}

/// Pack an array of u64s into `compressed` at the specified bit width. Caller
/// must ensure `compressed` is the right size
fn pack_bits(bit_width: u8, uncompressed: &[u64], mut compressed: &mut [u8]) {
	// We will use a `u64` as a mini buffer of 64 bits.
	// We accumulate bits in it until capacity, at which point we just copy this
	// mini buffer to compressed.
	let mut mini_buffer = 0u64;
	let mut remaining = 64;
	for el in uncompressed {
		mini_buffer |= el << (64 - remaining);
		if bit_width < remaining {
			remaining -= bit_width;
		} else {
			compressed[..8].copy_from_slice(&mini_buffer.to_le_bytes());
			compressed = &mut compressed[8..];
			mini_buffer = el >> remaining;
			remaining = 64 + remaining - bit_width;
		}
	}
	let mut remainder = compressed.len() % 8;
	if remainder == 0 {
		remainder = 8;
	}
	if mini_buffer > 0 {
		compressed[..].copy_from_slice(&mini_buffer.to_le_bytes()[..remainder]);
	}
}

fn extract_bits(bits: &[u8], bit_start: usize, bit_count: usize, read_from: usize) -> u64 {
	let mut buf: [u8; 8] = [0; 8];
	buf.copy_from_slice(&bits[read_from..read_from + 8]);
	if bit_count == 64 {
		return u64::from_le_bytes(buf);
	}
	let skip_bits = bit_start - read_from * 8;
	let bit_mask = (1 << bit_count) - 1;
	u64::from_le_bytes(buf) >> skip_bits & bit_mask
}

fn read_number(bits: &[u8], bit_start: usize, bit_count: usize) -> u64 {
	if bit_count == 0 {
		return 0;
	}
	// find where the first byte to read starts
	let mut read_from = bit_start / 8;
	// move back if we are too close to the end of bits
	if read_from + 8 > bits.len() {
		read_from = bits.len() - 8;
	}
	// calculate max bit we can read up to (+64 bits from the start)
	let max_bit_end = (read_from + 8) * 8;
	// calculate max bit we want to read
	let max_pos = bit_start + bit_count;
	// check if we can read it all at once
	if max_pos <= max_bit_end {
		extract_bits(bits, bit_start, bit_count, read_from)
	} else {
		let low = extract_bits(bits, bit_start, 8, read_from);
		let high = extract_bits(bits, bit_start + 8, bit_count - 8, read_from + 1);
		(high << 8) + low
	}
}

impl Readable for Proof {
	fn read<R: Reader>(reader: &mut R) -> Result<Proof, ser::Error> {
		let edge_bits = reader.read_u8()?;
		if edge_bits == 0 || edge_bits > 63 {
			return Err(ser::Error::CorruptedData);
		}

		// prepare nonces and read the right number of bytes
		// If skipping pow proof, we can stop after reading edge bits
		if reader.deserialization_mode() != DeserializationMode::SkipPow {
			let mut nonces = Vec::with_capacity(global::proofsize());
			let nonce_bits = edge_bits as usize;
			let bytes_len = Proof::pack_len(edge_bits);
			if bytes_len < 8 {
				return Err(ser::Error::CorruptedData);
			}
			let bits = reader.read_fixed_bytes(bytes_len)?;
			for n in 0..global::proofsize() {
				nonces.push(read_number(&bits, n * nonce_bits, nonce_bits));
			}

			//// check the last bits of the last byte are zeroed, we don't use them but
			//// still better to enforce to avoid any malleability
			let end_of_data = global::proofsize() * nonce_bits;
			if read_number(&bits, end_of_data, bytes_len * 8 - end_of_data) != 0 {
				return Err(ser::Error::CorruptedData);
			}
			Ok(Proof { edge_bits, nonces })
		} else {
			Ok(Proof {
				edge_bits,
				nonces: vec![],
			})
		}
	}
}

impl Writeable for Proof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		if writer.serialization_mode() != ser::SerializationMode::Hash {
			writer.write_u8(self.edge_bits)?;
		}
		writer.write_fixed_bytes(&self.pack_nonces())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ser::{BinReader, BinWriter, DeserializationMode, ProtocolVersion};
	use rand::Rng;
	use std::io::Cursor;

	#[test]
	fn test_proof_rw() {
		global::set_local_chain_type(global::ChainTypes::Mainnet);
		for edge_bits in 10..63 {
			let mut proof = Proof::new(gen_proof(edge_bits as u32));
			proof.edge_bits = edge_bits;
			let mut buf = Cursor::new(Vec::new());
			let mut w = BinWriter::new(&mut buf, ProtocolVersion::local());
			if let Err(e) = proof.write(&mut w) {
				panic!("failed to write proof {:?}", e);
			}
			buf.set_position(0);
			let mut r = BinReader::new(
				&mut buf,
				ProtocolVersion::local(),
				DeserializationMode::default(),
			);
			match Proof::read(&mut r) {
				Err(e) => panic!("failed to read proof: {:?}", e),
				Ok(p) => assert_eq!(p, proof),
			}
		}
	}

	fn gen_proof(bits: u32) -> Vec<u64> {
		let mut rng = rand::thread_rng();
		let mut v = Vec::with_capacity(42);
		for _ in 0..42 {
			v.push(rng.gen_range(
				u64::pow(2, bits - 1),
				if bits == 64 {
					std::u64::MAX
				} else {
					u64::pow(2, bits)
				},
			))
		}
		v
	}
}
