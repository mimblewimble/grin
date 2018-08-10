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

//! Core types

pub mod block;
pub mod committed;
pub mod hash;
pub mod id;
pub mod merkle_proof;
pub mod pmmr;
pub mod target;
pub mod transaction;

use consensus::GRIN_BASE;
#[allow(dead_code)]
use rand::{thread_rng, Rng};
use std::num::ParseFloatError;
use std::{fmt, iter};

use util::secp::pedersen::Commitment;

pub use self::block::*;
pub use self::committed::Committed;
pub use self::id::ShortId;
pub use self::transaction::*;
use core::hash::Hashed;
use global;
use ser::{self, Error, Readable, Reader, Writeable, Writer};

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

	/// Converts the proof to a proof-of-work Target so they can be compared.
	/// Hashes the Cuckoo Proof data.
	pub fn to_difficulty(&self) -> target::Difficulty {
		target::Difficulty::from_hash_and_shift(&self.hash(), self.cuckoo_sizeshift)
	}

	/// Returns the proof size
	pub fn proof_size(&self) -> usize {
		self.nonces.len()
	}
}

impl Readable for Proof {
	fn read(reader: &mut Reader) -> Result<Proof, Error> {
		let cuckoo_sizeshift = reader.read_u8()?;
		if cuckoo_sizeshift == 0 || cuckoo_sizeshift > 64 {
			return Err(Error::CorruptedData);
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
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
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

/// Common method for parsing an amount from human-readable, and converting
/// to internally-compatible u64

pub fn amount_from_hr_string(amount: &str) -> Result<u64, ParseFloatError> {
	let amount = amount.parse::<f64>()?;
	Ok((amount * GRIN_BASE as f64) as u64)
}

/// Common method for converting an amount to a human-readable string

pub fn amount_to_hr_string(amount: u64) -> String {
	let amount = (amount as f64 / GRIN_BASE as f64) as f64;
	let places = (GRIN_BASE as f64).log(10.0) as usize + 1;
	format!("{:.*}", places, amount)
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	pub fn test_amount_to_hr() {
		assert!(50123456789 == amount_from_hr_string("50.123456789").unwrap());
		assert!(50 == amount_from_hr_string(".000000050").unwrap());
		assert!(1 == amount_from_hr_string(".000000001").unwrap());
		assert!(0 == amount_from_hr_string(".0000000009").unwrap());
		assert!(500_000_000_000 == amount_from_hr_string("500").unwrap());
		assert!(
			5_000_000_000_000_000_000 == amount_from_hr_string("5000000000.00000000000").unwrap()
		);
	}

	#[test]
	pub fn test_hr_to_amount() {
		assert!("50.123456789" == amount_to_hr_string(50123456789));
		assert!("0.000000050" == amount_to_hr_string(50));
		assert!("0.000000001" == amount_to_hr_string(1));
		assert!("500.000000000" == amount_to_hr_string(500_000_000_000));
		assert!("5000000000.000000000" == amount_to_hr_string(5_000_000_000_000_000_000));
	}

}
