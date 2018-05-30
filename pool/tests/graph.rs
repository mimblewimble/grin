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

//! Top-level Graph tests

extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_pool as pool;
extern crate grin_wallet as wallet;

extern crate rand;

use core::core::OutputFeatures;
use core::core::transaction::ProofMessageElements;
use keychain::Keychain;
use wallet::libtx::proof;

#[test]
fn test_add_entry() {
	let keychain = Keychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();

	let output_commit = keychain.commit(70, &key_id1).unwrap();

	let inputs = vec![
		core::core::transaction::Input::new(
			OutputFeatures::DEFAULT_OUTPUT,
			keychain.commit(50, &key_id2).unwrap(),
			None,
			None,
		),
		core::core::transaction::Input::new(
			OutputFeatures::DEFAULT_OUTPUT,
			keychain.commit(25, &key_id3).unwrap(),
			None,
			None,
		),
	];

	let msg = ProofMessageElements::new(100, &key_id1);

	let output = core::core::transaction::Output {
		features: OutputFeatures::DEFAULT_OUTPUT,
		commit: output_commit,
		proof: proof::create(
			&keychain,
			100,
			&key_id1,
			output_commit,
			None,
			msg.to_proof_message(),
		).unwrap(),
	};

	let kernel = core::core::transaction::TxKernel::empty()
		.with_fee(5)
		.with_lock_height(0);

	let test_transaction =
		core::core::transaction::Transaction::new(inputs, vec![output], vec![kernel]);

	let test_pool_entry = pool::graph::PoolEntry::new(&test_transaction);

	let incoming_edge_1 = pool::graph::Edge::new(
		Some(random_hash()),
		Some(core::core::hash::ZERO_HASH),
		core::core::OutputIdentifier::from_output(&output),
	);

	let mut test_graph = pool::graph::DirectedGraph::empty();

	test_graph.add_entry(test_pool_entry, vec![incoming_edge_1]);

	assert_eq!(test_graph.vertices.len(), 1);
	assert_eq!(test_graph.roots.len(), 0);
	assert_eq!(test_graph.edges.len(), 1);
}

/// For testing/debugging: a random tx hash
fn random_hash() -> core::core::hash::Hash {
	let hash_bytes: [u8; 32] = rand::random();
	core::core::hash::Hash(hash_bytes)
}
