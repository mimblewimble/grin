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

use core::core::hash::Hash;
use core::core::pmmr::MerkleProof;
use core::core::{Input, Output, OutputFeatures, ProofMessageElements, Transaction, TxKernel};
use keychain::{self, BlindSum, BlindingFactor, Identifier, Keychain};
use libtx::{aggsig, proof};
use util::secp::pedersen::Commitment;
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
pub type Append<K: Keychain> = for<'a> Fn(&'a mut Context<K>, (Transaction, TxKernel, BlindSum))
	-> (Transaction, TxKernel, BlindSum);

/// Adds an input with the provided value and blinding key to the transaction
/// being built.
fn build_input<K>(
	value: u64,
	features: OutputFeatures,
	block_hash: Option<Hash>,
	merkle_proof: Option<MerkleProof>,
	key_id: Identifier,
) -> Box<Append<K>>
where
	K: Keychain,
{
	Box::new(
		move |build, (tx, kern, sum)| -> (Transaction, TxKernel, BlindSum) {
			let commit = build.keychain.commit(value, &key_id).unwrap();
			let input = Input::new(features, commit, block_hash.clone(), merkle_proof.clone());
			(tx.with_input(input), kern, sum.sub_key_id(key_id.clone()))
		},
	)
}

// "Spend" pair of outputs with a single aggregate key.
// We do not need to know the individual keys to be able to spend the pair.
fn entangled_inputs<K>(
	commit1: Commitment,
	commit2: Commitment,
	key: BlindingFactor,
) -> Box<Append<K>>
where
	K: Keychain,
{
	Box::new(
		move |build, (tx, kern, sum)| -> (Transaction, TxKernel, BlindSum) {
			let input1 = Input::new(OutputFeatures::DEFAULT_OUTPUT, commit1, None, None);
			let input2 = Input::new(OutputFeatures::DEFAULT_OUTPUT, commit2, None, None);
			(
				tx.with_input(input1).with_input(input2),
				kern,
				sum.sub_blinding_factor(key),
			)
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
		"Building input (spending regular output): {}, {}", value, key_id,
	);
	build_input(value, OutputFeatures::DEFAULT_OUTPUT, None, None, key_id)
}

/// Adds a coinbase input spending a coinbase output.
/// We will use the block hash to verify coinbase maturity.
pub fn coinbase_input<K>(
	value: u64,
	block_hash: Hash,
	merkle_proof: MerkleProof,
	key_id: Identifier,
) -> Box<Append<K>>
where
	K: Keychain,
{
	debug!(
		LOGGER,
		"Building input (spending coinbase): {}, {}", value, key_id
	);
	build_input(
		value,
		OutputFeatures::COINBASE_OUTPUT,
		Some(block_hash),
		Some(merkle_proof),
		key_id,
	)
}

/// Adds an output with the provided value and key identifier from the
/// keychain.
pub fn output<K>(value: u64, key_id: Identifier) -> Box<Append<K>>
where
	K: Keychain,
{
	Box::new(
		move |build, (tx, kern, sum)| -> (Transaction, TxKernel, BlindSum) {
			debug!(LOGGER, "Building an output: {}, {}", value, key_id,);

			let commit = build.keychain.commit(value, &key_id).unwrap();
			trace!(LOGGER, "Builder - Pedersen Commit is: {:?}", commit,);

			let msg = ProofMessageElements::new(value, &key_id);

			let rproof = proof::create(
				build.keychain,
				value,
				&key_id,
				commit,
				None,
				msg.to_proof_message(),
			).unwrap();

			(
				tx.with_output(Output {
					features: OutputFeatures::DEFAULT_OUTPUT,
					commit: commit,
					proof: rproof,
				}),
				kern,
				sum.add_key_id(key_id.clone()),
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
	assert_eq!(tx.kernels.len(), 1);
	let kern = tx.kernels.remove(0);
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
	let (mut tx, kern, sum) = elems.iter().fold(
		(Transaction::empty(), TxKernel::empty(), BlindSum::new()),
		|acc, elem| elem(&mut ctx, acc),
	);
	let blind_sum = ctx.keychain.blind_sum(&sum)?;

	// we only support building a tx with a single kernel via build::transaction()
	assert!(tx.kernels.is_empty());
	tx.kernels.push(kern);

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
	let (mut tx, blind_sum) = partial_transaction(elems, keychain)?;
	assert_eq!(tx.kernels.len(), 1);

	let mut kern = tx.kernels.remove(0);
	let msg = secp::Message::from_slice(&kernel_sig_msg(kern.fee, kern.lock_height))?;

	let skey = blind_sum.secret_key(&keychain.secp())?;
	kern.excess = keychain.secp().commit(0, skey)?;
	kern.excess_sig = aggsig::sign_with_blinding(&keychain.secp(), &msg, &blind_sum).unwrap();

	tx.kernels.push(kern);

	Ok(tx)
}

/// Builds a complete transaction, splitting the key and
/// setting the excess, excess_sig and tx offset as necessary.
pub fn transaction_with_offset<K>(
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

	let split = blind_sum.split(&keychain.secp())?;
	let k1 = split.blind_1;
	let k2 = split.blind_2;

	let msg = secp::Message::from_slice(&kernel_sig_msg(kern.fee, kern.lock_height))?;

	// generate kernel excess and excess_sig using the split key k1
	let skey = k1.secret_key(&keychain.secp())?;
	kern.excess = ctx.keychain.secp().commit(0, skey)?;
	kern.excess_sig = aggsig::sign_with_blinding(&keychain.secp(), &msg, &k1).unwrap();

	// store the kernel offset (k2) on the tx itself
	// commitments will sum correctly when including the offset
	tx.offset = k2.clone();

	assert!(tx.kernels.is_empty());
	tx.kernels.push(kern);

	Ok(tx)
}

// Just a simple test, most exhaustive tests in the core mod.rs.
#[cfg(test)]
mod test {
	use super::*;
	use keychain::ExtKeychain;

	#[test]
	fn blind_simple_tx() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let key_id3 = keychain.derive_key_id(3).unwrap();

		let tx = transaction(
			vec![
				input(10, key_id1),
				input(12, key_id2),
				output(20, key_id3),
				with_fee(2),
			],
			&keychain,
		).unwrap();

		tx.validate().unwrap();
	}

	#[test]
	fn blind_simple_tx_with_offset() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let key_id3 = keychain.derive_key_id(3).unwrap();

		let tx = transaction_with_offset(
			vec![
				input(10, key_id1),
				input(12, key_id2),
				output(20, key_id3),
				with_fee(2),
			],
			&keychain,
		).unwrap();

		tx.validate().unwrap();
	}

	#[test]
	fn blind_simpler_tx() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();

		let tx = transaction(
			vec![input(6, key_id1), output(2, key_id2), with_fee(4)],
			&keychain,
		).unwrap();

		tx.validate().unwrap();
	}

	#[test]
	fn test_entangled_outputs() {
		// using a keychain here for convenience but in reality
		// we would generate an initial key K and then split it into K=K1+K2
		// [tbd - can this be done such that K1 and K2 are never exposed?]
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();

		let key = {
			let key1 = keychain.derived_key(&key_id1).unwrap();
			let key2 = keychain.derived_key(&key_id2).unwrap();
			keychain.secp().blind_sum(vec![key1, key2], vec![]).unwrap()
		};

		// At this point key = key1 + key2 (where key1 and key2 are in our keychain for
		// convenience). But imagine we have a scenario where Alice and Bob interact
		// to construct these where neither knows either split keys and they only
		// know the single aggregate key. Neither party can spend Out1 or Out2
		// individually. Out1 and Out2 can only be spent together using the single
		// key.

		// Construct a tx that includes Out1 that we wish to encumber with a timelock.
		let key_id3 = keychain.derive_key_id(3).unwrap();
		let tx1 = transaction(
			vec![input(3, key_id3), output(2, key_id1.clone()), with_fee(1)],
			&keychain,
		).unwrap();
		assert!(tx1.validate().is_ok());

		// Now construct a second tx including Out2, but with a lock_height on the tx
		// itself.
		let key_id4 = keychain.derive_key_id(4).unwrap();
		let tx2 = transaction(
			vec![
				input(30, key_id4),
				output(20, key_id2.clone()),
				with_fee(10),
				with_lock_height(1_000),
			],
			&keychain,
		).unwrap();
		assert!(tx2.validate().is_ok());

		// These are the commitments from outputs 1 and 2 above.
		let commit1 = keychain.commit(2, &key_id1).unwrap();
		let commit2 = keychain.commit(20, &key_id2).unwrap();

		// At this point we have two outputs (using key1 and key2)

		// Now show we can spend the two outputs Out1 and Out2 _together_
		// if we know the two commitments and the original aggregate key.
		// Note: we can only do this _after_ the tx lock_height from Tx2 has passed.
		//
		// Out1 (confirmed and on-chain) can *only* be spent
		// after the lock_height specified in Tx2.
		//
		let blind = BlindingFactor::from_secret_key(key);
		let key_id5 = keychain.derive_key_id(5).unwrap();
		let tx3 = transaction(
			vec![
				entangled_inputs(commit1, commit2, blind),
				output(21, key_id5),
				with_fee(1),
			],
			&keychain,
		).unwrap();
		assert!(tx3.validate().is_ok());
	}
}
