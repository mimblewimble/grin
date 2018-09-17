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

use util::{kernel_sig_msg, secp};

use core::core::{Input, Output, OutputFeatures, Transaction, TxKernel};
use keychain::{self, BlindSum, BlindingFactor, Identifier, Keychain};
use libtx::{aggsig, proof};
use util::LOGGER;

/// Context information available to transaction combinators.
pub struct Context<'a, K: 'a>
where
	K: Keychain,
{
	keychain: &'a K,
}

/// Function type returned by the transaction combinators. Transforms a
/// (Transaction, BlindSum) pair into another, provided some context.
pub type Append<K> = for<'a> Fn(&'a mut Context<K>, (Transaction, TxKernel, BlindSum))
	-> (Transaction, TxKernel, BlindSum);

/// Adds an input with the provided value and blinding key to the transaction
/// being built.
fn build_input<K>(value: u64, features: OutputFeatures, key_id: Identifier) -> Box<Append<K>>
where
	K: Keychain,
{
	Box::new(
		move |build, (tx, kern, sum)| -> (Transaction, TxKernel, BlindSum) {
			let commit = build.keychain.commit(value, &key_id).unwrap();
			let input = Input::new(features, commit);
			(tx.with_input(input), kern, sum.sub_key_id(key_id.to_path()))
		},
	)
}

/// Adds an input with the provided value and blinding key to the transaction
/// being built.
pub fn input<K>(value: u64, key_id: Identifier) -> Box<Append<K>>
where
	K: Keychain,
{
	debug!(
		LOGGER,
		"Building input (spending regular output): {}, {}", value, key_id
	);
	build_input(value, OutputFeatures::DEFAULT_OUTPUT, key_id)
}

/// Adds a coinbase input spending a coinbase output.
pub fn coinbase_input<K>(value: u64, key_id: Identifier) -> Box<Append<K>>
where
	K: Keychain,
{
	debug!(
		LOGGER,
		"Building input (spending coinbase): {}, {}", value, key_id
	);
	build_input(value, OutputFeatures::COINBASE_OUTPUT, key_id)
}

/// Adds an output with the provided value and key identifier from the
/// keychain.
pub fn output<K>(value: u64, key_id: Identifier) -> Box<Append<K>>
where
	K: Keychain,
{
	Box::new(
		move |build, (tx, kern, sum)| -> (Transaction, TxKernel, BlindSum) {
			let commit = build.keychain.commit(value, &key_id).unwrap();

			debug!(LOGGER, "Building output: {}, {:?}", value, commit);

			let rproof = proof::create(build.keychain, value, &key_id, commit, None).unwrap();

			(
				tx.with_output(Output {
					features: OutputFeatures::DEFAULT_OUTPUT,
					commit: commit,
					proof: rproof,
				}),
				kern,
				sum.add_key_id(key_id.to_path()),
			)
		},
	)
}

/// Sets the fee on the transaction being built.
pub fn with_fee<K>(fee: u64) -> Box<Append<K>>
where
	K: Keychain,
{
	Box::new(
		move |_build, (tx, kern, sum)| -> (Transaction, TxKernel, BlindSum) {
			(tx, kern.with_fee(fee), sum)
		},
	)
}

/// Sets the lock_height on the transaction being built.
pub fn with_lock_height<K>(lock_height: u64) -> Box<Append<K>>
where
	K: Keychain,
{
	Box::new(
		move |_build, (tx, kern, sum)| -> (Transaction, TxKernel, BlindSum) {
			(tx, kern.with_lock_height(lock_height), sum)
		},
	)
}

/// Adds a known excess value on the transaction being built. Usually used in
/// combination with the initial_tx function when a new transaction is built
/// by adding to a pre-existing one.
pub fn with_excess<K>(excess: BlindingFactor) -> Box<Append<K>>
where
	K: Keychain,
{
	Box::new(
		move |_build, (tx, kern, sum)| -> (Transaction, TxKernel, BlindSum) {
			(tx, kern, sum.add_blinding_factor(excess.clone()))
		},
	)
}

/// Sets a known tx "offset". Used in final step of tx construction.
pub fn with_offset<K>(offset: BlindingFactor) -> Box<Append<K>>
where
	K: Keychain,
{
	Box::new(
		move |_build, (tx, kern, sum)| -> (Transaction, TxKernel, BlindSum) {
			(tx.with_offset(offset), kern, sum)
		},
	)
}

/// Sets an initial transaction to add to when building a new transaction.
/// We currently only support building a tx with a single kernel with
/// build::transaction()
pub fn initial_tx<K>(mut tx: Transaction) -> Box<Append<K>>
where
	K: Keychain,
{
	assert_eq!(tx.kernels().len(), 1);
	let kern = tx.kernels_mut().remove(0);
	Box::new(
		move |_build, (_, _, sum)| -> (Transaction, TxKernel, BlindSum) {
			(tx.clone(), kern.clone(), sum)
		},
	)
}

/// Builds a new transaction by combining all the combinators provided in a
/// Vector. Transactions can either be built "from scratch" with a list of
/// inputs or outputs or from a pre-existing transaction that gets added to.
///
/// Example:
/// let (tx1, sum) = build::transaction(vec![input_rand(4), output_rand(1),
///   with_fee(1)], keychain).unwrap();
/// let (tx2, _) = build::transaction(vec![initial_tx(tx1), with_excess(sum),
///   output_rand(2)], keychain).unwrap();
///
pub fn partial_transaction<K>(
	elems: Vec<Box<Append<K>>>,
	keychain: &K,
) -> Result<(Transaction, BlindingFactor), keychain::Error>
where
	K: Keychain,
{
	let mut ctx = Context { keychain };
	let (tx, kern, sum) = elems.iter().fold(
		(Transaction::empty(), TxKernel::empty(), BlindSum::new()),
		|acc, elem| elem(&mut ctx, acc),
	);
	let blind_sum = ctx.keychain.blind_sum(&sum)?;

	// we only support building a tx with a single kernel via build::transaction()
	assert!(tx.kernels().is_empty());

	let tx = tx.with_kernel(kern);

	Ok((tx, blind_sum))
}

/// Builds a complete transaction.
pub fn transaction<K>(
	elems: Vec<Box<Append<K>>>,
	keychain: &K,
) -> Result<Transaction, keychain::Error>
where
	K: Keychain,
{
	let mut ctx = Context { keychain };
	let (mut tx, mut kern, sum) = elems.iter().fold(
		(Transaction::empty(), TxKernel::empty(), BlindSum::new()),
		|acc, elem| elem(&mut ctx, acc),
	);
	let blind_sum = ctx.keychain.blind_sum(&sum)?;

	// Split the key so we can generate an offset for the tx.
	let split = blind_sum.split(&keychain.secp())?;
	let k1 = split.blind_1;
	let k2 = split.blind_2;

	// Construct the message to be signed.
	let msg = secp::Message::from_slice(&kernel_sig_msg(kern.fee, kern.lock_height))?;

	// Generate kernel excess and excess_sig using the split key k1.
	let skey = k1.secret_key(&keychain.secp())?;
	kern.excess = ctx.keychain.secp().commit(0, skey)?;
	let pubkey = &kern.excess.to_pubkey(&keychain.secp())?;
	kern.excess_sig =
		aggsig::sign_with_blinding(&keychain.secp(), &msg, &k1, Some(&pubkey)).unwrap();

	// Store the kernel offset (k2) on the tx.
	// Commitments will sum correctly when accounting for the offset.
	tx.offset = k2.clone();

	// Set the kernel on the tx (assert this is now a single-kernel tx).
	assert!(tx.kernels().is_empty());
	let tx = tx.with_kernel(kern);
	assert_eq!(tx.kernels().len(), 1);

	Ok(tx)
}

// Just a simple test, most exhaustive tests in the core mod.rs.
#[cfg(test)]
mod test {
	use std::sync::{Arc, RwLock};

	use super::*;
	use core::core::verifier_cache::{LruVerifierCache, VerifierCache};
	use keychain::{ExtKeychain, ExtKeychainPath};

	fn verifier_cache() -> Arc<RwLock<VerifierCache>> {
		Arc::new(RwLock::new(LruVerifierCache::new()))
	}

	#[test]
	fn blind_simple_tx() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
		let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
		let key_id3 = ExtKeychainPath::new(1, 3, 0, 0, 0).to_identifier();

		let vc = verifier_cache();

		let tx = transaction(
			vec![
				input(10, key_id1),
				input(12, key_id2),
				output(20, key_id3),
				with_fee(2),
			],
			&keychain,
		).unwrap();

		tx.validate(vc.clone()).unwrap();
	}

	#[test]
	fn blind_simple_tx_with_offset() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
		let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
		let key_id3 = ExtKeychainPath::new(1, 3, 0, 0, 0).to_identifier();

		let vc = verifier_cache();

		let tx = transaction(
			vec![
				input(10, key_id1),
				input(12, key_id2),
				output(20, key_id3),
				with_fee(2),
			],
			&keychain,
		).unwrap();

		tx.validate(vc.clone()).unwrap();
	}

	#[test]
	fn blind_simpler_tx() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
		let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();

		let vc = verifier_cache();

		let tx = transaction(
			vec![input(6, key_id1), output(2, key_id2), with_fee(4)],
			&keychain,
		).unwrap();

		tx.validate(vc.clone()).unwrap();
	}
}
