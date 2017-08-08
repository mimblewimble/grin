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
pub mod build;
pub mod hash;
pub mod pmmr;
pub mod sumtree;
pub mod target;
pub mod transaction;
//pub mod txoset;
#[allow(dead_code)]

use std::fmt;
use std::cmp::Ordering;

use secp::{self, Secp256k1};
use secp::pedersen::*;

pub use self::block::{Block, BlockHeader, DEFAULT_BLOCK};
pub use self::transaction::{Transaction, Input, Output, TxKernel, COINBASE_KERNEL,
                            COINBASE_OUTPUT, DEFAULT_OUTPUT};
use self::hash::{Hash, Hashed, ZERO_HASH};
use ser::{Writeable, Writer, Reader, Readable, Error};

use global;

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
		let mut input_commits = map_vec!(self.inputs_committed(), |inp| inp.commitment());
		let mut output_commits = map_vec!(self.outputs_committed(), |out| out.commitment());

		// add the overage as output commitment if positive, as an input commitment if
		// negative
		let overage = self.overage();
		if overage != 0 {
			let over_commit = secp.commit_value(overage.abs() as u64).unwrap();
			if overage < 0 {
				input_commits.push(over_commit);
			} else {
				output_commits.push(over_commit);
			}
		}

		// sum all that stuff
		secp.commit_sum(output_commits, input_commits)
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
pub struct Proof {
	pub nonces:Vec<u32>,
    pub proof_size: usize,
}

impl fmt::Debug for Proof {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		try!(write!(f, "Cuckoo("));
		for (i, val) in self.nonces[..].iter().enumerate() {
			try!(write!(f, "{:x}", val));
			if i < self.nonces.len() - 1 {
				try!(write!(f, " "));
			}
		}
		write!(f, ")")
	}
}
impl PartialOrd for Proof {
	fn partial_cmp(&self, other: &Proof) -> Option<Ordering> {
		self.nonces.partial_cmp(&other.nonces)
	}
}
impl PartialEq for Proof {
	fn eq(&self, other: &Proof) -> bool {
		self.nonces[..] == other.nonces[..]
	}
}
impl Eq for Proof {}
impl Clone for Proof {
	fn clone(&self) -> Proof {
		let mut out_nonces = Vec::new();
		for n in self.nonces.iter() {
			out_nonces.push(*n as u32);
		}
		Proof {
			proof_size: out_nonces.len(),
			nonces: out_nonces,
		}
	}
}

impl Proof {
	
	/// Builds a proof with all bytes zeroed out
	pub fn new(in_nonces:Vec<u32>) -> Proof {
		Proof {
			proof_size: in_nonces.len(),
			nonces: in_nonces,
		}
	}

	/// Builds a proof with all bytes zeroed out
	pub fn zero(proof_size:usize) -> Proof {
		Proof {
			proof_size: proof_size,
			nonces: vec![0;proof_size],
		}
	}

	/// Converts the proof to a vector of u64s
	pub fn to_u64s(&self) -> Vec<u64> {
		let mut out_nonces = Vec::with_capacity(self.proof_size);
		for n in self.nonces.iter() {
			out_nonces.push(*n as u64);
		}
		out_nonces
	}

	/// Converts the proof to a vector of u32s
	pub fn to_u32s(&self) -> Vec<u32> {
		self.clone().nonces
	}

	/// Converts the proof to a proof-of-work Target so they can be compared.
	/// Hashes the Cuckoo Proof data.
	pub fn to_difficulty(self) -> target::Difficulty {
		target::Difficulty::from_hash(&self.hash())
	}
}

impl Readable for Proof {
	fn read(reader: &mut Reader) -> Result<Proof, Error> {
		let proof_size = global::proofsize();
		let mut pow = vec![0u32; proof_size];
		for n in 0..proof_size {
			pow[n] = try!(reader.read_u32());
		}
		Ok(Proof::new(pow))
	}
}

impl Writeable for Proof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		for n in 0..self.proof_size {
			try!(writer.write_u32(self.nonces[n]));
		}
		Ok(())
	}
}

/// Two hashes that will get hashed together in a Merkle tree to build the next
/// level up.
struct HPair(Hash, Hash);

impl Writeable for HPair {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		try!(writer.write_bytes(&self.0));
		try!(writer.write_bytes(&self.1));
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
			[].hash()
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
	use ser;
	use rand::os::OsRng;
	use core::build::{self, input, output, input_rand, output_rand, with_fee, initial_tx,
	                  with_excess};

	fn new_secp() -> Secp256k1 {
		secp::Secp256k1::with_caps(secp::ContextFlag::Commit)
	}

	#[test]
	#[should_panic(expected = "InvalidSecretKey")]
	fn zero_commit() {
		// a transaction whose commitment sums to zero shouldn't validate
		let ref secp = new_secp();
		let mut rng = OsRng::new().unwrap();

		// blinding should fail as signing with a zero r*G shouldn't work
		let skey = SecretKey::new(secp, &mut rng);
		build::transaction(vec![input(10, skey), output(1, skey), with_fee(9)]).unwrap();
	}

	#[test]
	fn simple_tx_ser() {
		let tx = tx2i1o();
		let mut vec = Vec::new();
		ser::serialize(&mut vec, &tx).expect("serialized failed");
		assert!(vec.len() > 5320);
		assert!(vec.len() < 5340);
	}

	#[test]
	fn simple_tx_ser_deser() {
		let tx = tx2i1o();
		let mut vec = Vec::new();
		ser::serialize(&mut vec, &tx).expect("serialization failed");
		let dtx: Transaction = ser::deserialize(&mut &vec[..]).unwrap();
		assert_eq!(dtx.fee, 1);
		assert_eq!(dtx.inputs.len(), 2);
		assert_eq!(dtx.outputs.len(), 1);
		assert_eq!(tx.hash(), dtx.hash());
	}

	#[test]
	fn tx_double_ser_deser() {
		// checks serializing doesn't mess up the tx and produces consistent results
		let btx = tx2i1o();

		let mut vec = Vec::new();
		assert!(ser::serialize(&mut vec, &btx).is_ok());
		let dtx: Transaction = ser::deserialize(&mut &vec[..]).unwrap();

		let mut vec2 = Vec::new();
		assert!(ser::serialize(&mut vec2, &btx).is_ok());
		let dtx2: Transaction = ser::deserialize(&mut &vec2[..]).unwrap();

		assert_eq!(btx.hash(), dtx.hash());
		assert_eq!(dtx.hash(), dtx2.hash());
	}

	#[test]
	fn hash_output() {
		let (tx, _) =
			build::transaction(vec![input_rand(75), output_rand(42), output_rand(32), with_fee(1)])
				.unwrap();
		let h = tx.outputs[0].hash();
		assert!(h != ZERO_HASH);
		let h2 = tx.outputs[1].hash();
		assert!(h != h2);
	}

	#[test]
	fn blind_tx() {
		let ref secp = new_secp();

		let btx = tx2i1o();
		btx.verify_sig(&secp).unwrap(); // unwrap will panic if invalid

		// checks that the range proof on our blind output is sufficiently hiding
		let Output { proof, .. } = btx.outputs[0];
		let info = secp.range_proof_info(proof);
		assert!(info.min == 0);
		assert!(info.max == u64::max_value());
	}

	#[test]
	fn tx_hash_diff() {
		let btx1 = tx2i1o();
		let btx2 = tx1i1o();

		if btx1.hash() == btx2.hash() {
			panic!("diff txs have same hash")
		}
	}

	/// Simulate the standard exchange between 2 parties when creating a basic
	/// 2 inputs, 2 outputs transaction.
	#[test]
	fn tx_build_exchange() {
		let ref secp = new_secp();

		let tx_alice: Transaction;
		let blind_sum: SecretKey;

		{
			// Alice gets 2 of her pre-existing outputs to send 5 coins to Bob, they
			// become inputs in the new transaction
			let (in1, in2) = (input_rand(4), input_rand(3));

			// Alice builds her transaction, with change, which also produces the sum
			// of blinding factors before they're obscured.
			let (tx, sum) = build::transaction(vec![in1, in2, output_rand(1), with_fee(1)])
				.unwrap();
			tx_alice = tx;
			blind_sum = sum;
		}

		// From now on, Bob only has the obscured transaction and the sum of
		// blinding factors. He adds his output, finalizes the transaction so it's
		// ready for broadcast.
		let (tx_final, _) =
			build::transaction(vec![initial_tx(tx_alice), with_excess(blind_sum), output_rand(5)])
				.unwrap();

		tx_final.validate(&secp).unwrap();
	}

	#[test]
	fn reward_empty_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();
		let skey = SecretKey::new(secp, &mut rng);

		let b = Block::new(&BlockHeader::default(), vec![], skey).unwrap();
		b.compact().validate(&secp).unwrap();
	}

	#[test]
	fn reward_with_tx_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();
		let skey = SecretKey::new(secp, &mut rng);

		let mut tx1 = tx2i1o();
		tx1.verify_sig(&secp).unwrap();

		let b = Block::new(&BlockHeader::default(), vec![&mut tx1], skey).unwrap();
		b.compact().validate(&secp).unwrap();
	}

	#[test]
	fn simple_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();
		let skey = SecretKey::new(secp, &mut rng);

		let mut tx1 = tx2i1o();
		tx1.verify_sig(&secp).unwrap();

		let mut tx2 = tx1i1o();
		tx2.verify_sig(&secp).unwrap();

		let b = Block::new(&BlockHeader::default(), vec![&mut tx1, &mut tx2], skey).unwrap();
		b.validate(&secp).unwrap();
	}

	// utility producing a transaction with 2 inputs and a single outputs
	pub fn tx2i1o() -> Transaction {
		build::transaction(vec![input_rand(10), input_rand(11), output_rand(20), with_fee(1)])
			.map(|(tx, _)| tx)
			.unwrap()
	}

	// utility producing a transaction with a single input and output
	pub fn tx1i1o() -> Transaction {
		build::transaction(vec![input_rand(5), output_rand(4), with_fee(1)])
			.map(|(tx, _)| tx)
			.unwrap()
	}
}
