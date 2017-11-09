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
pub mod target;
pub mod transaction;
// pub mod txoset;
#[allow(dead_code)]

use std::fmt;
use std::cmp::Ordering;
use std::num::ParseFloatError;
use consensus::GRIN_BASE;

use util::{secp, static_secp_instance};
use util::secp::pedersen::*;

pub use self::block::*;
pub use self::transaction::*;
use self::hash::Hashed;
use ser::{Error, Readable, Reader, Writeable, Writer};
use global;

/// Implemented by types that hold inputs and outputs including Pedersen
/// commitments. Handles the collection of the commitments as well as their
/// summing, taking potential explicit overages of fees into account.
pub trait Committed {
	/// Gathers commitments and sum them.
	fn sum_commitments(&self) -> Result<Commitment, secp::Error> {
		// first, verify each range proof
		let ref outputs = self.outputs_committed();
		for output in *outputs {
			try!(output.verify_proof())
		}

		// then gather the commitments
		let mut input_commits = map_vec!(self.inputs_committed(), |inp| inp.commitment());
		let mut output_commits = map_vec!(self.outputs_committed(), |out| out.commitment());

		// add the overage as output commitment if positive, as an input commitment if
		// negative
		let overage = self.overage();
		if overage != 0 {
			let over_commit = {
				let secp = static_secp_instance();
				let secp = secp.lock().unwrap();
				secp.commit_value(overage.abs() as u64).unwrap()
			};
			if overage < 0 {
				input_commits.push(over_commit);
			} else {
				output_commits.push(over_commit);
			}
		}

		// sum all that stuff
		{
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			secp.commit_sum(output_commits, input_commits)
		}
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
	/// The nonces
	pub nonces: Vec<u32>,

	/// The proof size
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
	pub fn new(in_nonces: Vec<u32>) -> Proof {
		Proof {
			proof_size: in_nonces.len(),
			nonces: in_nonces,
		}
	}

	/// Builds a proof with all bytes zeroed out
	pub fn zero(proof_size: usize) -> Proof {
		Proof {
			proof_size: proof_size,
			nonces: vec![0; proof_size],
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
	String::from(format!("{:.*}", places, amount))
}

#[cfg(test)]
mod test {
	use super::*;
	use core::hash::ZERO_HASH;
	use core::build::{initial_tx, input, output, with_excess, with_fee, with_lock_height};
	use core::block::Error::KernelLockHeight;
	use ser;
	use keychain;
	use keychain::{BlindingFactor, Keychain};

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

	#[test]
	#[should_panic(expected = "InvalidSecretKey")]
	fn test_zero_commit_fails() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();

		// blinding should fail as signing with a zero r*G shouldn't work
		build::transaction(
			vec![
				input(10, key_id1.clone()),
				output(9, key_id1.clone()),
				with_fee(1),
			],
			&keychain,
		).unwrap();
	}

	#[test]
	fn simple_tx_ser() {
		let tx = tx2i1o();
		let mut vec = Vec::new();
		ser::serialize(&mut vec, &tx).expect("serialized failed");
		assert!(vec.len() > 5360);
		assert!(vec.len() < 5380);
	}

	#[test]
	fn simple_tx_ser_deser() {
		let tx = tx2i1o();
		let mut vec = Vec::new();
		ser::serialize(&mut vec, &tx).expect("serialization failed");
		let dtx: Transaction = ser::deserialize(&mut &vec[..]).unwrap();
		assert_eq!(dtx.fee, 2);
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
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let key_id3 = keychain.derive_key_id(3).unwrap();

		let (tx, _) = build::transaction(
			vec![
				input(75, key_id1),
				output(42, key_id2),
				output(32, key_id3),
				with_fee(1),
			],
			&keychain,
		).unwrap();
		let h = tx.outputs[0].hash();
		assert!(h != ZERO_HASH);
		let h2 = tx.outputs[1].hash();
		assert!(h != h2);
	}

	#[test]
	fn blind_tx() {
		let btx = tx2i1o();
		btx.verify_sig().unwrap(); // unwrap will panic if invalid

		// checks that the range proof on our blind output is sufficiently hiding
		let Output { proof, .. } = btx.outputs[0];

		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
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
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let key_id3 = keychain.derive_key_id(3).unwrap();
		let key_id4 = keychain.derive_key_id(4).unwrap();

		let tx_alice: Transaction;
		let blind_sum: BlindingFactor;

		{
			// Alice gets 2 of her pre-existing outputs to send 5 coins to Bob, they
			// become inputs in the new transaction
			let (in1, in2) = (input(4, key_id1), input(3, key_id2));

			// Alice builds her transaction, with change, which also produces the sum
			// of blinding factors before they're obscured.
			let (tx, sum) =
				build::transaction(vec![in1, in2, output(1, key_id3), with_fee(2)], &keychain)
					.unwrap();
			tx_alice = tx;
			blind_sum = sum;
		}

		// From now on, Bob only has the obscured transaction and the sum of
		// blinding factors. He adds his output, finalizes the transaction so it's
		// ready for broadcast.
		let (tx_final, _) = build::transaction(
			vec![
				initial_tx(tx_alice),
				with_excess(blind_sum),
				output(4, key_id4),
			],
			&keychain,
		).unwrap();

		tx_final.validate().unwrap();
	}

	#[test]
	fn reward_empty_block() {
		let keychain = keychain::Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();

		let b = Block::new(&BlockHeader::default(), vec![], &keychain, &key_id).unwrap();
		b.compact().validate().unwrap();
	}

	#[test]
	fn reward_with_tx_block() {
		let keychain = keychain::Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();

		let mut tx1 = tx2i1o();
		tx1.verify_sig().unwrap();

		let b = Block::new(&BlockHeader::default(), vec![&mut tx1], &keychain, &key_id).unwrap();
		b.compact().validate().unwrap();
	}

	#[test]
	fn simple_block() {
		let keychain = keychain::Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();

		let mut tx1 = tx2i1o();
		let mut tx2 = tx1i1o();

		let b = Block::new(
			&BlockHeader::default(),
			vec![&mut tx1, &mut tx2],
			&keychain,
			&key_id,
		).unwrap();
		b.validate().unwrap();
	}

	#[test]
	fn test_block_with_timelocked_tx() {
		let keychain = keychain::Keychain::from_random_seed().unwrap();

		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let key_id3 = keychain.derive_key_id(3).unwrap();

		// first check we can add a timelocked tx where lock height matches current block height
		// and that the resulting block is valid
		let tx1 = build::transaction(
			vec![
				input(5, key_id1.clone()),
				output(3, key_id2.clone()),
				with_fee(2),
				with_lock_height(1),
			],
			&keychain,
		).map(|(tx, _)| tx)
			.unwrap();

		let b = Block::new(
			&BlockHeader::default(),
			vec![&tx1],
			&keychain,
			&key_id3.clone(),
		).unwrap();
		b.validate().unwrap();

		// now try adding a timelocked tx where lock height is greater than current block height
		let tx1 = build::transaction(
			vec![
				input(5, key_id1.clone()),
				output(3, key_id2.clone()),
				with_fee(2),
				with_lock_height(2),
			],
			&keychain,
		).map(|(tx, _)| tx)
			.unwrap();

		let b = Block::new(
			&BlockHeader::default(),
			vec![&tx1],
			&keychain,
			&key_id3.clone(),
		).unwrap();
		match b.validate() {
			Err(KernelLockHeight { lock_height: height }) => {
				assert_eq!(height, 2);
			}
			_ => panic!("expecting KernelLockHeight error here"),
		}
	}

	#[test]
	pub fn test_verify_1i1o_sig() {
		let tx = tx1i1o();
		tx.verify_sig().unwrap();
	}

	#[test]
	pub fn test_verify_2i1o_sig() {
		let tx = tx2i1o();
		tx.verify_sig().unwrap();
	}

	// utility producing a transaction with 2 inputs and a single outputs
	pub fn tx2i1o() -> Transaction {
		let keychain = keychain::Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let key_id3 = keychain.derive_key_id(3).unwrap();

		build::transaction(
			vec![
				input(10, key_id1),
				input(11, key_id2),
				output(19, key_id3),
				with_fee(2),
			],
			&keychain,
		).map(|(tx, _)| tx)
			.unwrap()
	}

	// utility producing a transaction with a single input and output
	pub fn tx1i1o() -> Transaction {
		let keychain = keychain::Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();

		build::transaction(
			vec![input(5, key_id1), output(3, key_id2), with_fee(2)],
			&keychain,
		).map(|(tx, _)| tx)
			.unwrap()
	}
}
