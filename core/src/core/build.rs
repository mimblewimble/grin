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

//! Utility functions to build Grin transactions. Handles the blinding of
//! inputs and outputs, maintaining the sum of blinding factors, producing
//! the excess signature, etc.
//!
//! Each building function is a combinator that produces a function taking
//! a transaction a sum of blinding factors, to return another transaction
//! and sum. Combinators can then be chained and executed using the
//! _transaction_ function.
//!
//! Example:
//! build::transaction(vec![input_rand(75), output_rand(42), output_rand(32),
//!   with_fee(1)])

use byteorder::{ByteOrder, BigEndian};
use secp::{self, Secp256k1};
use secp::key::SecretKey;
use rand::os::OsRng;

use core::{Transaction, Input, Output, DEFAULT_OUTPUT};

/// Context information available to transaction combinators.
pub struct Context {
	secp: Secp256k1,
	rng: OsRng,
}

/// Accumulator to compute the sum of blinding factors. Keeps track of each
/// factor as well as the "sign" with which they should be combined.
pub struct BlindSum {
	positive: Vec<SecretKey>,
	negative: Vec<SecretKey>,
}

impl BlindSum {
	/// Creates a new blinding factor sum.
	fn new() -> BlindSum {
		BlindSum {
			positive: vec![],
			negative: vec![],
		}
	}

	/// Adds the provided key to the sum of blinding factors.
	fn add(self, key: SecretKey) -> BlindSum {
		let mut new_pos = self.positive;
		new_pos.push(key);
		BlindSum {
			positive: new_pos,
			negative: self.negative,
		}
	}

	/// Subtractss the provided key to the sum of blinding factors.
	fn sub(self, key: SecretKey) -> BlindSum {
		let mut new_neg = self.negative;
		new_neg.push(key);
		BlindSum {
			positive: self.positive,
			negative: new_neg,
		}
	}

	/// Computes the sum of blinding factors from all the ones that have been
	/// added and subtracted.
	fn sum(self, secp: &Secp256k1) -> Result<SecretKey, secp::Error> {
		secp.blind_sum(self.positive, self.negative)
	}
}

/// Function type returned by the transaction combinators. Transforms a
/// (Transaction, BlindSum) pair into another, provided some context.
type Append = for<'a> Fn(&'a mut Context, (Transaction, BlindSum)) -> (Transaction, BlindSum);

/// Adds an input with the provided value and blinding key to the transaction
/// being built.
pub fn input(value: u64, blinding: SecretKey) -> Box<Append> {
	Box::new(move |build, (tx, sum)| -> (Transaction, BlindSum) {
		let commit = build.secp.commit(value, blinding).unwrap();
		(tx.with_input(Input(commit)), sum.sub(blinding))
	})
}

/// Adds an input with the provided value and a randomly generated blinding
/// key to the transaction being built. This has no real use in practical
/// applications but is very convenient for tests.
pub fn input_rand(value: u64) -> Box<Append> {
	Box::new(move |build, (tx, sum)| -> (Transaction, BlindSum) {
		let blinding = SecretKey::new(&build.secp, &mut build.rng);
		let commit = build.secp.commit(value, blinding).unwrap();
		(tx.with_input(Input(commit)), sum.sub(blinding))
	})
}

/// Adds an output with the provided value and blinding key to the transaction
/// being built.
pub fn output(value: u64, blinding: SecretKey) -> Box<Append> {
	Box::new(move |build, (tx, sum)| -> (Transaction, BlindSum) {
		let commit = build.secp.commit(value, blinding).unwrap();
		let nonce = build.secp.nonce();
		let rproof = build.secp.range_proof(0, value, blinding, commit, nonce);
		(tx.with_output(Output {
			features: DEFAULT_OUTPUT,
			commit: commit,
			proof: rproof,
		}),
		 sum.add(blinding))
	})
}

/// Adds an output with the provided value and a randomly generated blinding
/// key to the transaction being built. This has no real use in practical
/// applications but is very convenient for tests.
pub fn output_rand(value: u64) -> Box<Append> {
	Box::new(move |build, (tx, sum)| -> (Transaction, BlindSum) {
		let blinding = SecretKey::new(&build.secp, &mut build.rng);
		let commit = build.secp.commit(value, blinding).unwrap();
		let nonce = build.secp.nonce();
		let rproof = build.secp.range_proof(0, value, blinding, commit, nonce);
		(tx.with_output(Output {
			features: DEFAULT_OUTPUT,
			commit: commit,
			proof: rproof,
		}),
		 sum.add(blinding))
	})
}

/// Sets the fee on the transaction being built.
pub fn with_fee(fee: u64) -> Box<Append> {
	Box::new(move |_build, (tx, sum)| -> (Transaction, BlindSum) { (tx.with_fee(fee), sum) })
}

/// Sets a known excess value on the transaction being built. Usually used in
/// combination with the initial_tx function when a new transaction is built
/// by adding to a pre-existing one.
pub fn with_excess(excess: SecretKey) -> Box<Append> {
	Box::new(move |_build, (tx, sum)| -> (Transaction, BlindSum) { (tx, sum.add(excess)) })
}

/// Sets an initial transaction to add to when building a new transaction.
pub fn initial_tx(tx: Transaction) -> Box<Append> {
	Box::new(move |_build, (_, sum)| -> (Transaction, BlindSum) { (tx.clone(), sum) })
}

/// Builds a new transaction by combining all the combinators provided in a
/// Vector. Transactions can either be built "from scratch" with a list of
/// inputs or outputs or from a pre-existing transaction that gets added to.
///
/// Example:
/// let (tx1, sum) = build::transaction(vec![input_rand(4), output_rand(1),
///   with_fee(1)]).unwrap();
/// let (tx2, _) = build::transaction(vec![initial_tx(tx1), with_excess(sum),
///   output_rand(2)]).unwrap();
///
pub fn transaction(elems: Vec<Box<Append>>) -> Result<(Transaction, SecretKey), secp::Error> {
	let mut ctx = Context {
		secp: Secp256k1::with_caps(secp::ContextFlag::Commit),
		rng: OsRng::new().unwrap(),
	};
	let (mut tx, sum) = elems.iter().fold((Transaction::empty(), BlindSum::new()),
	                                      |acc, elem| elem(&mut ctx, acc));

	let blind_sum = sum.sum(&ctx.secp)?;
	let msg = secp::Message::from_slice(&u64_to_32bytes(tx.fee))?;
	let sig = ctx.secp.sign(&msg, &blind_sum)?;
	tx.excess_sig = sig.serialize_der(&ctx.secp);

	Ok((tx, blind_sum))
}

fn u64_to_32bytes(n: u64) -> [u8; 32] {
	let mut bytes = [0; 32];
	BigEndian::write_u64(&mut bytes[24..32], n);
	bytes
}


// Just a simple test, most exhaustive tests in the core mod.rs.
#[cfg(test)]
mod test {
	use super::*;

	use secp::{self, key, Secp256k1};

	#[test]
	fn blind_simple_tx() {
		let secp = Secp256k1::with_caps(secp::ContextFlag::Commit);
		let (tx, _) =
			transaction(vec![input_rand(10), input_rand(11), output_rand(20), with_fee(1)])
				.unwrap();
		tx.verify_sig(&secp).unwrap();
	}
	#[test]
	fn blind_simpler_tx() {
		let secp = Secp256k1::with_caps(secp::ContextFlag::Commit);
		let (tx, _) = transaction(vec![input_rand(6), output(2, key::ONE_KEY), with_fee(4)])
			.unwrap();
		tx.verify_sig(&secp).unwrap();
	}
}
