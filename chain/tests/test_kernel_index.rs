// Copyright 2020 The Grin Developers
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

use self::chain::Chain;
use self::core::core::hash::Hashed;
use self::core::core::{Block, BlockHeader, Transaction};
use self::core::global::ChainTypes;
use self::core::libtx;
use self::core::pow::Difficulty;
use self::core::{global, pow};
use self::keychain::{ExtKeychain, ExtKeychainPath, Keychain};
use chrono::Duration;
use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;

mod chain_test_helper;

use self::chain_test_helper::{clean_output_dir, init_chain};

#[test]
fn kernel_index_after_compaction() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	util::init_test_logger();
	// Cleanup chain directory
	let chain_dir = ".grin_kernel_idx";
	clean_output_dir(chain_dir);

	let chain = init_chain(chain_dir, pow::mine_genesis_block().unwrap());
	let mut prev = chain.head_header().unwrap();
	let kc = ExtKeychain::from_random_seed(false).unwrap();

	// mine some blocks
	for n in 0..30 {
		let next = prepare_block(&kc, &prev, &chain, 10 + n);
		prev = next.header.clone();
		chain.process_block(next, chain::Options::SKIP_POW).unwrap();
	}

	chain.validate(false).unwrap();

	{
		let head = chain.head().unwrap();
		let header_at_horizon = chain
			.get_header_by_height(
				head.height
					.saturating_sub(global::kernel_index_horizon() as u64),
			)
			.unwrap();
		let block_at_horizon = chain.get_block(&header_at_horizon.hash()).unwrap();
		let block_pre_horizon = chain.get_block(&header_at_horizon.prev_hash).unwrap();

		// Chain compaction will remove all blocks earlier than the horizon.
		chain.compact().expect("chain compaction error");

		// Kernels up to and including the horizon must be in the kernel index.
		let kernel = block_at_horizon.kernels().first().unwrap();
		chain.get_kernel_pos(kernel.excess).unwrap();

		// Kernels beyond the horizon are no longer in the kernel index.
		let kernel = block_pre_horizon.kernels().first().unwrap();
		chain
			.get_kernel_pos(kernel.excess)
			.expect_err("kernel_pos should be compacted");
	}

	// Cleanup chain directory
	clean_output_dir(chain_dir);
}

fn prepare_block<K>(kc: &K, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block
where
	K: Keychain,
{
	let mut b = prepare_block_nosum(kc, prev, diff, vec![]);
	chain.set_txhashset_roots(&mut b).unwrap();
	b
}

fn prepare_block_nosum<K>(kc: &K, prev: &BlockHeader, diff: u64, txs: Vec<&Transaction>) -> Block
where
	K: Keychain,
{
	let proof_size = global::proofsize();
	let key_id = ExtKeychainPath::new(1, diff as u32, 0, 0, 0).to_identifier();

	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let reward =
		libtx::reward::output(kc, &libtx::ProofBuilder::new(kc), &key_id, fees, false).unwrap();
	let mut b = match core::core::Block::new(
		prev,
		txs.into_iter().cloned().collect(),
		Difficulty::from_num(diff),
		reward,
	) {
		Err(e) => panic!("{:?}", e),
		Ok(b) => b,
	};
	b.header.timestamp = prev.timestamp + Duration::seconds(60);
	b.header.pow.total_difficulty = prev.total_difficulty() + Difficulty::from_num(diff);
	b.header.pow.proof = pow::Proof::random(proof_size);
	b
}
