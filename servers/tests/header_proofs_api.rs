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

//! `get_header_proofs` from the Foreign API down to a client-side check, on
//! a short mined chain.

use std::sync::{Arc, Weak};

use grin_api::{Error, Foreign, HeaderProofsPrintable};
use grin_chain::types::{NoopAdapter, Options};
use grin_chain::Chain;
use grin_core::core::hash::Hashed;
use grin_core::core::header_proof::{self, Level};
use grin_core::core::{Block, BlockHeader};
use grin_core::libtx::{self, reward};
use grin_core::{consensus, genesis, global, pow};
use grin_keychain::{ExtKeychain, ExtKeychainPath, Keychain};
use grin_pool::types::NoopPoolAdapter;
use grin_servers::common::adapters::PoolToChainAdapter;
use grin_util::ToHex;

fn mined_chain(dir: &str, length: u64) -> Arc<Chain> {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	let _ = std::fs::remove_dir_all(dir);
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = libtx::ProofBuilder::new(&keychain);
	let key_id = ExtKeychain::derive_key_id(0, 1, 0, 0, 0);
	let (output, kernel) = reward::output(&keychain, &builder, &key_id, 0, false).unwrap();
	let genesis = genesis::genesis_dev().with_reward(output, kernel);
	let chain = Chain::init(
		dir.to_owned(),
		Arc::new(NoopAdapter {}),
		genesis,
		pow::verify_size,
		false,
		None,
	)
	.unwrap();
	for n in 1..length {
		let prev = chain.head_header().unwrap();
		let next = consensus::next_difficulty(prev.height + 1, chain.difficulty_iter().unwrap());
		let key_id = ExtKeychainPath::new(1, n as u32, 0, 0, 0).to_identifier();
		let reward = reward::output(&keychain, &builder, &key_id, 0, false).unwrap();
		let mut block = Block::new(&prev, &[], next.difficulty, reward).unwrap();
		block.header.timestamp = prev.timestamp + chrono::Duration::seconds(60);
		block.header.pow.secondary_scaling = next.secondary_scaling;
		chain.set_txhashset_roots(&mut block).unwrap();
		pow::pow_size(
			&mut block.header,
			next.difficulty,
			global::proofsize(),
			global::min_edge_bits(),
		)
		.unwrap();
		chain.process_block(block, Options::MINE).unwrap();
	}
	Arc::new(chain)
}

fn foreign(chain: &Arc<Chain>) -> Foreign<PoolToChainAdapter, NoopPoolAdapter> {
	Foreign::new(Arc::downgrade(chain), Weak::new(), Weak::new())
}

#[test]
fn a_remote_wallet_checks_the_nodes_headers() {
	let dir = ".grin.header_proofs_api";
	let chain = mined_chain(dir, 20);
	let api = foreign(&chain);
	let anchor: BlockHeader = chain.get_header_by_height(18).unwrap();

	// What a wallet receives is JSON.
	let answer = api
		.get_header_proofs(anchor.hash().to_hex(), vec![5, 13, 17], None)
		.unwrap();
	let json = serde_json::to_string(&answer).unwrap();
	let received: HeaderProofsPrintable = serde_json::from_str(&json).unwrap();
	let proofs = received.to_proofs().unwrap();
	assert_eq!(
		header_proof::verify(&anchor, &proofs).unwrap(),
		vec![Level::Pow, Level::Exact, Level::Exact]
	);

	// A node that alters a header it serves is caught.
	let mut altered = received.clone();
	let mut header = altered.to_proofs().unwrap().items[1].header.clone();
	header.output_root = header.kernel_root;
	altered.items[1].header =
		grin_core::ser::ser_vec(&header, grin_core::ser::ProtocolVersion::local())
			.unwrap()
			.to_hex();
	assert!(header_proof::verify(&anchor, &altered.to_proofs().unwrap()).is_err());
	let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn refusals_carry_a_reason() {
	let dir = ".grin.header_proofs_api_refused";
	let chain = mined_chain(dir, 12);
	let api = foreign(&chain);
	let anchor = chain.get_header_by_height(10).unwrap().hash().to_hex();

	assert!(matches!(
		api.get_header_proofs("00".repeat(32), vec![3], None),
		Err(Error::NotFound)
	));
	assert!(matches!(
		api.get_header_proofs(anchor.clone(), vec![10], None),
		Err(Error::Argument(_))
	));
	assert!(matches!(
		api.get_header_proofs(anchor.clone(), vec![4, 3], None),
		Err(Error::Argument(_))
	));
	assert!(matches!(
		api.get_header_proofs(anchor, (0..65).collect(), Some(0)),
		Err(Error::Argument(_))
	));

	let mut side = chain.get_header_by_height(8).unwrap();
	side.pow.proof.nonces[0] += 1;
	{
		let store = chain.store();
		let mut batch = store.batch().unwrap();
		batch.save_block_header(&side).unwrap();
		batch.commit().unwrap();
	}
	match api.get_header_proofs(side.hash().to_hex(), vec![3], None) {
		Err(Error::Argument(reason)) => assert!(reason.contains("not on the main header chain")),
		other => panic!("unexpected {:?}", other),
	}
	let _ = std::fs::remove_dir_all(dir);
}
