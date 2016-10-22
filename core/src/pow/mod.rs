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

//! The proof of work needs to strike a balance between fast header
//! verification to avoid DoS attacks and difficulty for block verifiers to
//! build new blocks. In addition, mining new blocks should also be as
//! difficult on high end custom-made hardware (ASICs) as on commodity hardware
//! or smartphones. For this reason we use Cuckoo Cycle (see the cuckoo
//! module for more information).
//!
//! Note that this miner implementation is here mostly for tests and
//! reference. It's not optimized for speed.

mod siphash;
mod cuckoo;

use time;

use core::{Block, BlockHeader, Proof, PROOFSIZE};
use core::hash::{Hash, Hashed};
use pow::cuckoo::{Cuckoo, Miner, Error};

use ser;
use ser::{Writeable, Writer, ser_vec};

/// Default Cuckoo Cycle size shift used is 28. We may decide to increase it.
/// when difficuty increases.
const SIZESHIFT: u32 = 28;

/// Default Cuckoo Cycle easiness, high enough to have good likeliness to find
/// a solution.
const EASINESS: u32 = 50;

/// Max target hash, lowest difficulty
pub const MAX_TARGET: [u32; PROOFSIZE] =
	[0xfff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff,
	 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff,
	 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff,
	 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff];

/// Subset of a block header that goes into hashing for proof of work.
/// Basically the whole thing minus the PoW solution itself and the total
/// difficulty (yet unknown). We also add the count of every variable length
/// elements in a header to make lying on those much harder.
#[derive(Debug)]
struct PowHeader {
	pub nonce: u64,
	pub height: u64,
	pub previous: Hash,
	pub timestamp: time::Tm,
	pub utxo_merkle: Hash,
	pub tx_merkle: Hash,
	pub total_fees: u64,
	pub n_in: u64,
	pub n_out: u64,
	pub n_proofs: u64,
}

/// The binary definition of a PoW header is material for consensus as that's
/// the data that gets hashed for PoW calculation. The nonce is written first
/// to make incrementing from the serialized form trivial.
impl Writeable for PowHeader {
	fn write(&self, writer: &mut Writer) -> Option<ser::Error> {
		try_m!(writer.write_u64(self.nonce));
		try_m!(writer.write_u64(self.height));
		try_m!(writer.write_fixed_bytes(&self.previous));
		try_m!(writer.write_i64(self.timestamp.to_timespec().sec));
		try_m!(writer.write_fixed_bytes(&self.utxo_merkle));
		try_m!(writer.write_fixed_bytes(&self.tx_merkle));
		try_m!(writer.write_u64(self.total_fees));
		try_m!(writer.write_u64(self.n_in));
		try_m!(writer.write_u64(self.n_out));
		writer.write_u64(self.n_proofs)
	}
}

impl Hashed for PowHeader {
	fn bytes(&self) -> Vec<u8> {
		// no serialization errors are applicable in this specific case
		ser_vec(self).unwrap()
	}
}

impl PowHeader {
	fn from_block(b: &Block) -> PowHeader {
		let ref h = b.header;
		PowHeader {
			nonce: h.nonce,
			height: h.height,
			previous: h.previous,
			timestamp: h.timestamp,
			utxo_merkle: h.utxo_merkle,
			tx_merkle: h.tx_merkle,
			total_fees: h.total_fees,
			n_in: b.inputs.len() as u64,
			n_out: b.outputs.len() as u64,
			n_proofs: b.proofs.len() as u64,
		}
	}
}

/// Validates the proof of work of a given header.
pub fn verify(b: &Block, target: Proof) -> bool {
	verify_size(b, target, SIZESHIFT)
}

/// Same as default verify function but uses the much easier Cuckoo20 (mostly
/// for tests).
pub fn verify20(b: &Block, target: Proof) -> bool {
	verify_size(b, target, 20)
}

pub fn verify_size(b: &Block, target: Proof, sizeshift: u32) -> bool {
	let hash = PowHeader::from_block(b).hash();
	// make sure the hash is smaller than our target before going into more
	// expensive validation
	if target < b.header.pow {
		return false;
	}
	Cuckoo::new(hash.to_slice(), sizeshift).verify(b.header.pow, EASINESS as u64)
}

/// Runs a naive single-threaded proof of work computation over the provided
/// block, until the required difficulty target is reached. May take a
/// while for a low target...
pub fn pow(b: &Block, target: Proof) -> Result<(Proof, u64), Error> {
	pow_size(b, target, SIZESHIFT)
}

/// Same as default pow function but uses the much easier Cuckoo20 (mostly for
/// tests).
pub fn pow20(b: &Block, target: Proof) -> Result<(Proof, u64), Error> {
	pow_size(b, target, 20)
}

fn pow_size(b: &Block, target: Proof, sizeshift: u32) -> Result<(Proof, u64), Error> {
	let mut pow_header = PowHeader::from_block(b);
	let start_nonce = pow_header.nonce;

	// try to find a cuckoo cycle on that header hash
	loop {
		// can be trivially optimized by avoiding re-serialization every time but this
		// is not meant as a fast miner implementation
		let pow_hash = pow_header.hash();

		// if we found a cycle (not guaranteed) and the proof is lower that the target,
		// we're all good
		if let Ok(proof) = Miner::new(pow_hash.to_slice(), EASINESS, sizeshift).mine() {
			if proof <= target {
				return Ok((proof, pow_header.nonce));
			}
		}

		// otherwise increment the nonce
		pow_header.nonce += 1;

		// and if we're back where we started, update the time (changes the hash as
		// well)
		if pow_header.nonce == start_nonce {
			pow_header.timestamp = time::at_utc(time::Timespec { sec: 0, nsec: 0 });
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use core::{BlockHeader, Proof};
	use core::hash::Hash;
	use std::time::Instant;
	use genesis;

	#[test]
	fn genesis_pow() {
		let mut b = genesis::genesis();
		let (proof, nonce) = pow20(&b, Proof(MAX_TARGET)).unwrap();
		assert!(nonce > 0);
		assert!(proof < Proof(MAX_TARGET));
		b.header.pow = proof;
		b.header.nonce = nonce;
		assert!(verify20(&b, Proof(MAX_TARGET)));
	}
}
