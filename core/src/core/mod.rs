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

//! Core types

pub mod block;
pub mod hash;
pub mod target;
pub mod transaction;
#[allow(dead_code)]

use std::fmt;
use std::cmp::Ordering;

use secp::{self, Secp256k1};
use secp::pedersen::*;

use consensus::PROOFSIZE;
pub use self::block::{Block, BlockHeader};
pub use self::transaction::{Transaction, Input, Output, TxProof};
use self::hash::{Hash, Hashed, HashWriter, ZERO_HASH};
use ser::{Writeable, Writer, Reader, Readable, Error};

/// Implemented by types that hold inputs and outputs including Pedersen
/// commitments. Handles the collection of the commitments as well as their
/// summing, taking potential explicit overages of fees into account.
pub trait Committed {
	/// Gathers commitments and sum them.
	fn sum_commitments(&self, secp: &Secp256k1) -> Result<Commitment, secp::Error> {
		// first, verify each range proof
		let ref outputs = self.outputs_committed();
		for output in *outputs {
			try!(output.verify_proof(secp))
		}

		// then gather the commitments
		let mut input_commits = filter_map_vec!(self.inputs_committed(), |inp| inp.commitment());
		let mut output_commits = filter_map_vec!(self.outputs_committed(), |out| out.commitment());

		// add the overage as input commitment if positive, as an output commitment if
		// negative
		let overage = self.overage();
		if overage != 0 {
			let over_commit = secp.commit_value(overage.abs() as u64).unwrap();
			if overage > 0 {
				input_commits.push(over_commit);
			} else {
				output_commits.push(over_commit);
			}
		}

		// sum all that stuff
		secp.commit_sum(input_commits, output_commits)
	}

	/// Vector of committed inputs to verify
	fn inputs_committed(&self) -> &Vec<Input>;

	/// Vector of committed inputs to verify
	fn outputs_committed(&self) -> &Vec<Output>;

	/// The overage amount expected over the commitments. Can be negative (a
	/// fee) or positive (a reward).
	fn overage(&self) -> i64;
}


/// Proof of work
#[derive(Copy)]
pub struct Proof(pub [u32; PROOFSIZE]);

impl fmt::Debug for Proof {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		try!(write!(f, "Cuckoo("));
		for (i, val) in self.0[..].iter().enumerate() {
			try!(write!(f, "{:x}", val));
			if i < PROOFSIZE - 1 {
				write!(f, " ");
			}
		}
		write!(f, ")")
	}
}
impl PartialOrd for Proof {
	fn partial_cmp(&self, other: &Proof) -> Option<Ordering> {
		self.0.partial_cmp(&other.0)
	}
}
impl PartialEq for Proof {
	fn eq(&self, other: &Proof) -> bool {
		self.0[..] == other.0[..]
	}
}
impl Eq for Proof {}
impl Clone for Proof {
	fn clone(&self) -> Proof {
		*self
	}
}

impl Proof {
	/// Builds a proof with all bytes zeroed out
	pub fn zero() -> Proof {
		Proof([0; PROOFSIZE])
	}

	/// Converts the proof to a vector of u64s
	pub fn to_u64s(&self) -> Vec<u64> {
		let mut nonces = Vec::with_capacity(PROOFSIZE);
		for n in self.0.iter() {
			nonces.push(*n as u64);
		}
		nonces
	}

	/// Converts the proof to a vector of u32s
	pub fn to_u32s(&self) -> Vec<u32> {
		self.0.to_vec()
	}

	/// Converts the proof to a proof-of-work Target so they can be compared.
	/// Hashes the Cuckoo Proof data.
	pub fn to_difficulty(self) -> target::Difficulty {
		target::Difficulty::from_hash(&self.hash())
	}
}

impl Readable<Proof> for Proof {
	fn read(reader: &mut Reader) -> Result<Proof, Error> {
		let mut pow = [0u32; PROOFSIZE];
		for n in 0..PROOFSIZE {
			pow[n] = try!(reader.read_u32());
		}
		Ok(Proof(pow))
	}
}

impl Writeable for Proof {
	fn write(&self, writer: &mut Writer) -> Result<(), Error> {
		for n in 0..PROOFSIZE {
			try!(writer.write_u32(self.0[n]));
		}
		Ok(())
	}
}

/// Two hashes that will get hashed together in a Merkle tree to build the next
/// level up.
struct HPair(Hash, Hash);

impl Writeable for HPair {
	fn write(&self, writer: &mut Writer) -> Result<(), Error> {
		try!(writer.write_bytes(&self.0.to_slice()));
		try!(writer.write_bytes(&self.1.to_slice()));
		Ok(())
	}
}
/// An iterator over hashes in a vector that pairs them to build a row in a
/// Merkle tree. If the vector has an odd number of hashes, it appends a zero
/// hash
/// See https://bitcointalk.org/index.php?topic=102395.0 CVE-2012-2459 (block
/// merkle calculation exploit)
/// for the argument against duplication of last hash
struct HPairIter(Vec<Hash>);
impl Iterator for HPairIter {
	type Item = HPair;

	fn next(&mut self) -> Option<HPair> {
		self.0.pop().map(|first| HPair(first, self.0.pop().unwrap_or(ZERO_HASH)))
	}
}
/// A row in a Merkle tree. Can be built from a vector of hashes. Calculates
/// the next level up, or can recursively go all the way up to its root.
struct MerkleRow(Vec<HPair>);
impl MerkleRow {
	fn new(hs: Vec<Hash>) -> MerkleRow {
		MerkleRow(HPairIter(hs).map(|hp| hp).collect())
	}
	fn up(&self) -> MerkleRow {
		MerkleRow::new(map_vec!(self.0, |hp| hp.hash()))
	}
	fn root(&self) -> Hash {
		if self.0.len() == 0 {
			vec![].hash()
		} else if self.0.len() == 1 {
			self.0[0].hash()
		} else {
			self.up().root()
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use core::hash::ZERO_HASH;
	use secp;
	use secp::Secp256k1;
	use secp::key::SecretKey;
	use rand::Rng;
	use rand::os::OsRng;

	fn new_secp() -> Secp256k1 {
		secp::Secp256k1::with_caps(secp::ContextFlag::Commit)
	}

	#[test]
	#[should_panic(expected = "InvalidSecretKey")]
	fn zero_commit() {
		// a transaction whose commitment sums to zero shouldn't validate
		let ref secp = new_secp();
		let mut rng = OsRng::new().unwrap();

		let skey = SecretKey::new(secp, &mut rng);
		let outh = ZERO_HASH;
		let tx = Transaction::new(vec![Input::OvertInput {
			                               output: outh,
			                               value: 10,
			                               blindkey: skey,
		                               }],
		                          vec![Output::OvertOutput {
			                               value: 1,
			                               blindkey: skey,
		                               }],
		                          9);
		// blinding should fail as signing with a zero r*G shouldn't work
		tx.blind(&secp).unwrap();
	}

	#[test]
	fn reward_empty_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();
		let skey = SecretKey::new(secp, &mut rng);

		let b = Block::new(&BlockHeader::default(), vec![], skey).unwrap();
		b.compact().verify(&secp).unwrap();
	}

	#[test]
	fn reward_with_tx_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();
		let skey = SecretKey::new(secp, &mut rng);

		let tx1 = tx2i1o(secp, &mut rng);
		let mut btx1 = tx1.blind(&secp).unwrap();
		btx1.verify_sig(&secp).unwrap();

		let b = Block::new(&BlockHeader::default(), vec![&mut btx1], skey).unwrap();
		b.compact().verify(&secp).unwrap();
	}

	#[test]
	fn simple_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();
		let skey = SecretKey::new(secp, &mut rng);

		let tx1 = tx2i1o(secp, &mut rng);
		let mut btx1 = tx1.blind(&secp).unwrap();
		btx1.verify_sig(&secp).unwrap();

		let tx2 = tx1i1o(secp, &mut rng);
		let mut btx2 = tx2.blind(&secp).unwrap();
		btx2.verify_sig(&secp).unwrap();

		let b = Block::new(&BlockHeader::default(), vec![&mut btx1, &mut btx2], skey).unwrap();
		b.verify(&secp).unwrap();
	}

	// utility producing a transaction with 2 inputs and a single outputs
	pub fn tx2i1o<R: Rng>(secp: &Secp256k1, rng: &mut R) -> Transaction {
		let outh = ZERO_HASH;
		Transaction::new(vec![Input::OvertInput {
			                      output: outh,
			                      value: 10,
			                      blindkey: SecretKey::new(secp, rng),
		                      },
		                      Input::OvertInput {
			                      output: outh,
			                      value: 11,
			                      blindkey: SecretKey::new(secp, rng),
		                      }],
		                 vec![Output::OvertOutput {
			                      value: 20,
			                      blindkey: SecretKey::new(secp, rng),
		                      }],
		                 1)
	}

	// utility producing a transaction with a single input and output
	pub fn tx1i1o<R: Rng>(secp: &Secp256k1, rng: &mut R) -> Transaction {
		let outh = ZERO_HASH;
		Transaction::new(vec![Input::OvertInput {
			                      output: outh,
			                      value: 5,
			                      blindkey: SecretKey::new(secp, rng),
		                      }],
		                 vec![Output::OvertOutput {
			                      value: 4,
			                      blindkey: SecretKey::new(secp, rng),
		                      }],
		                 1)
	}
}
