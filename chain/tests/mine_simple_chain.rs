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

use self::chain::types::{NoopAdapter, Tip};
use self::chain::Chain;
use self::core::core::hash::Hashed;
use self::core::core::{
	block, transaction, Block, BlockHeader, KernelFeatures, Output, OutputFeatures, Transaction,
};
use self::core::global::ChainTypes;
use self::core::libtx::build::{self, Append};
use self::core::libtx::proof::{self, ProofBuild};
use self::core::libtx::{self, Error, ProofBuilder};
use self::core::pow::Difficulty;
use self::core::{consensus, global, pow};
use self::keychain::{
	BlindSum, ExtKeychain, ExtKeychainPath, Identifier, Keychain, SwitchCommitmentType,
};
use self::util::RwLock;
use chrono::Duration;
use grin_chain as chain;
use grin_chain::{BlockStatus, ChainAdapter, Options};
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;
use std::sync::Arc;

mod chain_test_helper;

use self::chain_test_helper::{clean_output_dir, init_chain, mine_chain};

/// Adapter to retrieve last status
pub struct StatusAdapter {
	pub last_status: RwLock<Option<BlockStatus>>,
}

impl StatusAdapter {
	pub fn new(last_status: RwLock<Option<BlockStatus>>) -> Self {
		StatusAdapter { last_status }
	}
}

impl ChainAdapter for StatusAdapter {
	fn block_accepted(&self, _b: &Block, status: BlockStatus, _opts: Options) {
		*self.last_status.write() = Some(status);
	}
}

/// Creates a `Chain` instance with `StatusAdapter` attached to it.
fn setup_with_status_adapter(dir_name: &str, genesis: Block, adapter: Arc<StatusAdapter>) -> Chain {
	util::init_test_logger();
	clean_output_dir(dir_name);
	let chain = chain::Chain::init(
		dir_name.to_string(),
		adapter,
		genesis,
		pow::verify_size,
		false,
	)
	.unwrap();

	chain
}

#[test]
fn mine_empty_chain() {
	let chain_dir = ".grin.empty";
	clean_output_dir(chain_dir);
	let chain = mine_chain(chain_dir, 1);
	assert_eq!(chain.head().unwrap().height, 0);
	clean_output_dir(chain_dir);
}

#[test]
fn mine_short_chain() {
	let chain_dir = ".grin.short";
	clean_output_dir(chain_dir);
	let chain = mine_chain(chain_dir, 4);
	assert_eq!(chain.head().unwrap().height, 3);
	clean_output_dir(chain_dir);
}

// Convenience wrapper for processing a full block on the test chain.
fn process_header(chain: &Chain, header: &BlockHeader) {
	chain
		.process_block_header(header, chain::Options::SKIP_POW)
		.unwrap();
}

// Convenience wrapper for processing a block header on the test chain.
fn process_block(chain: &Chain, block: &Block) {
	chain
		.process_block(block.clone(), chain::Options::SKIP_POW)
		.unwrap();
}

//
// a - b - c
//  \
//   - b'
//
// Process in the following order -
// 1. block_a
// 2. block_b
// 3. block_b'
// 4. header_c
// 5. block_c
//
#[test]
fn test_block_a_block_b_block_b_fork_header_c_fork_block_c() {
	let chain_dir = ".grin.block_a_block_b_block_b_fork_header_c_fork_block_c";
	clean_output_dir(chain_dir);
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	let kc = ExtKeychain::from_random_seed(false).unwrap();
	let genesis = pow::mine_genesis_block().unwrap();
	let last_status = RwLock::new(None);
	let adapter = Arc::new(StatusAdapter::new(last_status));
	let chain = setup_with_status_adapter(chain_dir, genesis.clone(), adapter.clone());

	let block_a = prepare_block(&kc, &chain.head_header().unwrap(), &chain, 1);
	process_block(&chain, &block_a);

	let block_b = prepare_block(&kc, &block_a.header, &chain, 2);
	let block_b_fork = prepare_block(&kc, &block_a.header, &chain, 2);

	process_block(&chain, &block_b);
	process_block(&chain, &block_b_fork);

	let block_c = prepare_block(&kc, &block_b.header, &chain, 3);
	process_header(&chain, &block_c.header);

	assert_eq!(chain.head().unwrap(), Tip::from_header(&block_b.header));
	assert_eq!(
		chain.header_head().unwrap(),
		Tip::from_header(&block_c.header)
	);

	process_block(&chain, &block_c);

	assert_eq!(chain.head().unwrap(), Tip::from_header(&block_c.header));
	assert_eq!(
		chain.header_head().unwrap(),
		Tip::from_header(&block_c.header)
	);

	clean_output_dir(chain_dir);
}

//
// a - b
//  \
//   - b' - c'
//
// Process in the following order -
// 1. block_a
// 2. block_b
// 3. block_b'
// 4. header_c'
// 5. block_c'
//
#[test]
fn test_block_a_block_b_block_b_fork_header_c_fork_block_c_fork() {
	let chain_dir = ".grin.block_a_block_b_block_b_fork_header_c_fork_block_c_fork";
	clean_output_dir(chain_dir);
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	let kc = ExtKeychain::from_random_seed(false).unwrap();
	let genesis = pow::mine_genesis_block().unwrap();
	let last_status = RwLock::new(None);
	let adapter = Arc::new(StatusAdapter::new(last_status));
	let chain = setup_with_status_adapter(chain_dir, genesis.clone(), adapter.clone());

	let block_a = prepare_block(&kc, &chain.head_header().unwrap(), &chain, 1);
	process_block(&chain, &block_a);

	let block_b = prepare_block(&kc, &block_a.header, &chain, 2);
	let block_b_fork = prepare_block(&kc, &block_a.header, &chain, 2);

	process_block(&chain, &block_b);
	process_block(&chain, &block_b_fork);

	let block_c_fork = prepare_block(&kc, &block_b_fork.header, &chain, 3);
	process_header(&chain, &block_c_fork.header);

	assert_eq!(chain.head().unwrap(), Tip::from_header(&block_b.header));
	assert_eq!(
		chain.header_head().unwrap(),
		Tip::from_header(&block_c_fork.header)
	);

	process_block(&chain, &block_c_fork);

	assert_eq!(
		chain.head().unwrap(),
		Tip::from_header(&block_c_fork.header)
	);
	assert_eq!(
		chain.header_head().unwrap(),
		Tip::from_header(&block_c_fork.header)
	);

	clean_output_dir(chain_dir);
}

//
// a - b - c
//  \
//   - b'
//
// Process in the following order -
// 1. block_a
// 2. header_b
// 3. header_b_fork
// 4. block_b_fork
// 5. block_b
// 6. block_c
//
#[test]
fn test_block_a_header_b_header_b_fork_block_b_fork_block_b_block_c() {
	let chain_dir = ".grin.test_block_a_header_b_header_b_fork_block_b_fork_block_b_block_c";
	clean_output_dir(chain_dir);
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	let kc = ExtKeychain::from_random_seed(false).unwrap();
	let genesis = pow::mine_genesis_block().unwrap();
	let last_status = RwLock::new(None);
	let adapter = Arc::new(StatusAdapter::new(last_status));
	let chain = setup_with_status_adapter(chain_dir, genesis.clone(), adapter.clone());

	let block_a = prepare_block(&kc, &chain.head_header().unwrap(), &chain, 1);
	process_block(&chain, &block_a);

	let block_b = prepare_block(&kc, &block_a.header, &chain, 2);
	let block_b_fork = prepare_block(&kc, &block_a.header, &chain, 2);

	process_header(&chain, &block_b.header);
	process_header(&chain, &block_b_fork.header);
	process_block(&chain, &block_b_fork);
	process_block(&chain, &block_b);

	assert_eq!(
		chain.header_head().unwrap(),
		Tip::from_header(&block_b.header)
	);
	assert_eq!(
		chain.head().unwrap(),
		Tip::from_header(&block_b_fork.header)
	);

	let block_c = prepare_block(&kc, &block_b.header, &chain, 3);
	process_block(&chain, &block_c);

	assert_eq!(chain.head().unwrap(), Tip::from_header(&block_c.header));
	assert_eq!(
		chain.header_head().unwrap(),
		Tip::from_header(&block_c.header)
	);

	clean_output_dir(chain_dir);
}

//
// a - b
//  \
//   - b' - c'
//
// Process in the following order -
// 1. block_a
// 2. header_b
// 3. header_b_fork
// 4. block_b_fork
// 5. block_b
// 6. block_c_fork
//
#[test]
fn test_block_a_header_b_header_b_fork_block_b_fork_block_b_block_c_fork() {
	let chain_dir = ".grin.test_block_a_header_b_header_b_fork_block_b_fork_block_b_block_c_fork";
	clean_output_dir(chain_dir);
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	let kc = ExtKeychain::from_random_seed(false).unwrap();
	let genesis = pow::mine_genesis_block().unwrap();
	let last_status = RwLock::new(None);
	let adapter = Arc::new(StatusAdapter::new(last_status));
	let chain = setup_with_status_adapter(chain_dir, genesis.clone(), adapter.clone());

	let block_a = prepare_block(&kc, &chain.head_header().unwrap(), &chain, 1);
	process_block(&chain, &block_a);

	let block_b = prepare_block(&kc, &block_a.header, &chain, 2);
	let block_b_fork = prepare_block(&kc, &block_a.header, &chain, 2);

	process_header(&chain, &block_b.header);
	process_header(&chain, &block_b_fork.header);
	process_block(&chain, &block_b_fork);
	process_block(&chain, &block_b);

	assert_eq!(
		chain.header_head().unwrap(),
		Tip::from_header(&block_b.header)
	);
	assert_eq!(
		chain.head().unwrap(),
		Tip::from_header(&block_b_fork.header)
	);

	let block_c_fork = prepare_block(&kc, &block_b_fork.header, &chain, 3);
	process_block(&chain, &block_c_fork);

	assert_eq!(
		chain.head().unwrap(),
		Tip::from_header(&block_c_fork.header)
	);
	assert_eq!(
		chain.header_head().unwrap(),
		Tip::from_header(&block_c_fork.header)
	);

	clean_output_dir(chain_dir);
}

#[test]
// This test creates a reorg at REORG_DEPTH by mining a block with difficulty that
// exceeds original chain total difficulty.
//
// Illustration of reorg with NUM_BLOCKS_MAIN = 6 and REORG_DEPTH = 5:
//
// difficulty:    1        2        3        4        5        6
//
//                       / [ 2  ] - [ 3  ] - [ 4  ] - [ 5  ] - [ 6  ] <- original chain
// [ Genesis ] -[ 1 ]- *
//                     ^ \ [ 2' ] - ................................  <- reorg chain with depth 5
//                     |
// difficulty:    1    |   24
//                     |
//                     \----< Fork point and chain reorg
fn mine_reorg() {
	// Test configuration
	const NUM_BLOCKS_MAIN: u64 = 6; // Number of blocks to mine in main chain
	const REORG_DEPTH: u64 = 5; // Number of blocks to be discarded from main chain after reorg

	const DIR_NAME: &str = ".grin_reorg";
	clean_output_dir(DIR_NAME);

	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	let kc = ExtKeychain::from_random_seed(false).unwrap();

	let genesis = pow::mine_genesis_block().unwrap();
	{
		// Create chain that reports last block status
		let last_status = RwLock::new(None);
		let adapter = Arc::new(StatusAdapter::new(last_status));
		let chain = setup_with_status_adapter(DIR_NAME, genesis.clone(), adapter.clone());

		// Add blocks to main chain with gradually increasing difficulty
		let mut prev = chain.head_header().unwrap();
		for n in 1..=NUM_BLOCKS_MAIN {
			let b = prepare_block(&kc, &prev, &chain, n);
			prev = b.header.clone();
			chain.process_block(b, chain::Options::SKIP_POW).unwrap();
		}

		let head = chain.head().unwrap();
		assert_eq!(head.height, NUM_BLOCKS_MAIN);
		assert_eq!(head.hash(), prev.hash());

		// Reorg chain should exceed main chain's total difficulty to be considered
		let reorg_difficulty = head.total_difficulty.to_num();

		// Create one block for reorg chain forking off NUM_BLOCKS_MAIN - REORG_DEPTH height
		let fork_head = chain
			.get_header_by_height(NUM_BLOCKS_MAIN - REORG_DEPTH)
			.unwrap();
		let b = prepare_block(&kc, &fork_head, &chain, reorg_difficulty);
		let reorg_head = b.header.clone();
		chain.process_block(b, chain::Options::SKIP_POW).unwrap();

		// Check that reorg is correctly reported in block status
		let fork_point = chain.get_header_by_height(1).unwrap();
		assert_eq!(
			*adapter.last_status.read(),
			Some(BlockStatus::Reorg {
				prev: Tip::from_header(&fork_head),
				prev_head: head,
				fork_point: Tip::from_header(&fork_point)
			})
		);

		// Chain should be switched to the reorganized chain
		let head = chain.head().unwrap();
		assert_eq!(head.height, NUM_BLOCKS_MAIN - REORG_DEPTH + 1);
		assert_eq!(head.hash(), reorg_head.hash());
	}

	// Cleanup chain directory
	clean_output_dir(DIR_NAME);
}

#[test]
fn mine_forks() {
	clean_output_dir(".grin2");
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	{
		let chain = init_chain(".grin2", pow::mine_genesis_block().unwrap());
		let kc = ExtKeychain::from_random_seed(false).unwrap();

		// add a first block to not fork genesis
		let prev = chain.head_header().unwrap();
		let b = prepare_block(&kc, &prev, &chain, 2);
		chain.process_block(b, chain::Options::SKIP_POW).unwrap();

		// mine and add a few blocks

		for n in 1..4 {
			// first block for one branch
			let prev = chain.head_header().unwrap();
			let b1 = prepare_block(&kc, &prev, &chain, 3 * n);

			// process the first block to extend the chain
			let bhash = b1.hash();
			chain.process_block(b1, chain::Options::SKIP_POW).unwrap();

			// checking our new head
			let head = chain.head().unwrap();
			assert_eq!(head.height, (n + 1) as u64);
			assert_eq!(head.last_block_h, bhash);
			assert_eq!(head.prev_block_h, prev.hash());

			// 2nd block with higher difficulty for other branch
			let b2 = prepare_block(&kc, &prev, &chain, 3 * n + 1);

			// process the 2nd block to build a fork with more work
			let bhash = b2.hash();
			chain.process_block(b2, chain::Options::SKIP_POW).unwrap();

			// checking head switch
			let head = chain.head().unwrap();
			assert_eq!(head.height, (n + 1) as u64);
			assert_eq!(head.last_block_h, bhash);
			assert_eq!(head.prev_block_h, prev.hash());
		}
	}
	// Cleanup chain directory
	clean_output_dir(".grin2");
}

#[test]
fn mine_losing_fork() {
	clean_output_dir(".grin3");
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	let kc = ExtKeychain::from_random_seed(false).unwrap();
	{
		let chain = init_chain(".grin3", pow::mine_genesis_block().unwrap());

		// add a first block we'll be forking from
		let prev = chain.head_header().unwrap();
		let b1 = prepare_block(&kc, &prev, &chain, 2);
		let b1head = b1.header.clone();
		chain.process_block(b1, chain::Options::SKIP_POW).unwrap();

		// prepare the 2 successor, sibling blocks, one with lower diff
		let b2 = prepare_block(&kc, &b1head, &chain, 4);
		let b2head = b2.header.clone();
		let bfork = prepare_block(&kc, &b1head, &chain, 3);

		// add higher difficulty first, prepare its successor, then fork
		// with lower diff
		chain.process_block(b2, chain::Options::SKIP_POW).unwrap();
		assert_eq!(chain.head_header().unwrap().hash(), b2head.hash());
		let b3 = prepare_block(&kc, &b2head, &chain, 5);
		chain
			.process_block(bfork, chain::Options::SKIP_POW)
			.unwrap();

		// adding the successor
		let b3head = b3.header.clone();
		chain.process_block(b3, chain::Options::SKIP_POW).unwrap();
		assert_eq!(chain.head_header().unwrap().hash(), b3head.hash());
	}
	// Cleanup chain directory
	clean_output_dir(".grin3");
}

#[test]
fn longer_fork() {
	clean_output_dir(".grin4");
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	let kc = ExtKeychain::from_random_seed(false).unwrap();
	// to make it easier to compute the txhashset roots in the test, we
	// prepare 2 chains, the 2nd will be have the forked blocks we can
	// then send back on the 1st
	let genesis = pow::mine_genesis_block().unwrap();
	{
		let chain = init_chain(".grin4", genesis.clone());

		// add blocks to both chains, 20 on the main one, only the first 5
		// for the forked chain
		let mut prev = chain.head_header().unwrap();
		for n in 0..10 {
			let b = prepare_block(&kc, &prev, &chain, 2 * n + 2);
			prev = b.header.clone();
			chain.process_block(b, chain::Options::SKIP_POW).unwrap();
		}

		let forked_block = chain.get_header_by_height(5).unwrap();

		let head = chain.head_header().unwrap();
		assert_eq!(head.height, 10);
		assert_eq!(head.hash(), prev.hash());

		let mut prev = forked_block;
		for n in 0..7 {
			let b = prepare_block(&kc, &prev, &chain, 2 * n + 11);
			prev = b.header.clone();
			chain.process_block(b, chain::Options::SKIP_POW).unwrap();
		}

		let new_head = prev;

		// After all this the chain should have switched to the fork.
		let head = chain.head_header().unwrap();
		assert_eq!(head.height, 12);
		assert_eq!(head.hash(), new_head.hash());
	}
	// Cleanup chain directory
	clean_output_dir(".grin4");
}

#[test]
fn spend_rewind_spend() {
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	util::init_test_logger();
	let chain_dir = ".grin_spend_rewind_spend";
	clean_output_dir(chain_dir);

	{
		let chain = init_chain(chain_dir, pow::mine_genesis_block().unwrap());
		let prev = chain.head_header().unwrap();
		let kc = ExtKeychain::from_random_seed(false).unwrap();
		let pb = ProofBuilder::new(&kc);

		let mut head = prev;

		// mine the first block and keep track of the block_hash
		// so we can spend the coinbase later
		let b = prepare_block_key_idx(&kc, &head, &chain, 2, 1);
		assert!(b.outputs()[0].is_coinbase());
		head = b.header.clone();
		chain
			.process_block(b.clone(), chain::Options::SKIP_POW)
			.unwrap();

		// now mine three further blocks
		for n in 3..6 {
			let b = prepare_block(&kc, &head, &chain, n);
			head = b.header.clone();
			chain.process_block(b, chain::Options::SKIP_POW).unwrap();
		}

		// Make a note of this header as we will rewind back to here later.
		let rewind_to = head.clone();

		let key_id_coinbase = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
		let key_id30 = ExtKeychainPath::new(1, 30, 0, 0, 0).to_identifier();

		let tx1 = build::transaction(
			KernelFeatures::Plain { fee: 20000.into() },
			&[
				build::coinbase_input(consensus::REWARD, key_id_coinbase.clone()),
				build::output(consensus::REWARD - 20000, key_id30.clone()),
			],
			&kc,
			&pb,
		)
		.unwrap();

		let b = prepare_block_tx(&kc, &head, &chain, 6, &[tx1.clone()]);
		head = b.header.clone();
		chain
			.process_block(b.clone(), chain::Options::SKIP_POW)
			.unwrap();
		chain.validate(false).unwrap();

		// Now mine another block, reusing the private key for the coinbase we just spent.
		{
			let b = prepare_block_key_idx(&kc, &head, &chain, 7, 1);
			chain.process_block(b, chain::Options::SKIP_POW).unwrap();
		}

		// Now mine a competing block also spending the same coinbase output from earlier.
		// Rewind back prior to the tx that spends it to "unspend" it.
		{
			let b = prepare_block_tx(&kc, &rewind_to, &chain, 6, &[tx1]);
			chain
				.process_block(b.clone(), chain::Options::SKIP_POW)
				.unwrap();
			chain.validate(false).unwrap();
		}
	}

	clean_output_dir(chain_dir);
}

#[test]
fn spend_in_fork_and_compact() {
	clean_output_dir(".grin6");
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	util::init_test_logger();
	{
		let chain = init_chain(".grin6", pow::mine_genesis_block().unwrap());
		let prev = chain.head_header().unwrap();
		let kc = ExtKeychain::from_random_seed(false).unwrap();
		let pb = ProofBuilder::new(&kc);

		let mut fork_head = prev;

		// mine the first block and keep track of the block_hash
		// so we can spend the coinbase later
		let b = prepare_block(&kc, &fork_head, &chain, 2);
		assert!(b.outputs()[0].is_coinbase());
		fork_head = b.header.clone();
		chain
			.process_block(b.clone(), chain::Options::SKIP_POW)
			.unwrap();

		// now mine three further blocks
		for n in 3..6 {
			let b = prepare_block(&kc, &fork_head, &chain, n);
			fork_head = b.header.clone();
			chain.process_block(b, chain::Options::SKIP_POW).unwrap();
		}

		// Check the height of the "fork block".
		assert_eq!(fork_head.height, 4);
		let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
		let key_id30 = ExtKeychainPath::new(1, 30, 0, 0, 0).to_identifier();
		let key_id31 = ExtKeychainPath::new(1, 31, 0, 0, 0).to_identifier();

		let tx1 = build::transaction(
			KernelFeatures::Plain { fee: 20000.into() },
			&[
				build::coinbase_input(consensus::REWARD, key_id2.clone()),
				build::output(consensus::REWARD - 20000, key_id30.clone()),
			],
			&kc,
			&pb,
		)
		.unwrap();

		let next = prepare_block_tx(&kc, &fork_head, &chain, 7, &[tx1.clone()]);
		let prev_main = next.header.clone();
		chain
			.process_block(next.clone(), chain::Options::SKIP_POW)
			.unwrap();
		chain.validate(false).unwrap();

		let tx2 = build::transaction(
			KernelFeatures::Plain { fee: 20000.into() },
			&[
				build::input(consensus::REWARD - 20000, key_id30.clone()),
				build::output(consensus::REWARD - 40000, key_id31.clone()),
			],
			&kc,
			&pb,
		)
		.unwrap();

		let next = prepare_block_tx(&kc, &prev_main, &chain, 9, &[tx2.clone()]);
		let prev_main = next.header.clone();
		chain.process_block(next, chain::Options::SKIP_POW).unwrap();

		// Full chain validation for completeness.
		chain.validate(false).unwrap();

		// mine 2 forked blocks from the first
		let fork = prepare_block_tx(&kc, &fork_head, &chain, 6, &[tx1.clone()]);
		let prev_fork = fork.header.clone();
		chain.process_block(fork, chain::Options::SKIP_POW).unwrap();

		let fork_next = prepare_block_tx(&kc, &prev_fork, &chain, 8, &[tx2.clone()]);
		let prev_fork = fork_next.header.clone();
		chain
			.process_block(fork_next, chain::Options::SKIP_POW)
			.unwrap();

		chain.validate(false).unwrap();

		// check state
		let head = chain.head_header().unwrap();
		assert_eq!(head.height, 6);
		assert_eq!(head.hash(), prev_main.hash());
		assert!(chain
			.get_unspent(tx2.outputs()[0].commitment())
			.unwrap()
			.is_some());
		assert!(chain
			.get_unspent(tx1.outputs()[0].commitment())
			.unwrap()
			.is_none());

		// make the fork win
		let fork_next = prepare_block(&kc, &prev_fork, &chain, 10);
		let prev_fork = fork_next.header.clone();
		chain
			.process_block(fork_next, chain::Options::SKIP_POW)
			.unwrap();
		chain.validate(false).unwrap();

		// check state
		let head = chain.head_header().unwrap();
		assert_eq!(head.height, 7);
		assert_eq!(head.hash(), prev_fork.hash());
		assert!(chain
			.get_unspent(tx2.outputs()[0].commitment())
			.unwrap()
			.is_some());
		assert!(chain
			.get_unspent(tx1.outputs()[0].commitment())
			.unwrap()
			.is_none());

		// add 20 blocks to go past the test horizon
		let mut prev = prev_fork;
		for n in 0..20 {
			let next = prepare_block(&kc, &prev, &chain, 11 + n);
			prev = next.header.clone();
			chain.process_block(next, chain::Options::SKIP_POW).unwrap();
		}

		chain.validate(false).unwrap();
		if let Err(e) = chain.compact() {
			panic!("Error compacting chain: {:?}", e);
		}
		if let Err(e) = chain.validate(false) {
			panic!("Validation error after compacting chain: {:?}", e);
		}
	}
	// Cleanup chain directory
	clean_output_dir(".grin6");
}

/// Test ability to retrieve block headers for a given output
#[test]
fn output_header_mappings() {
	clean_output_dir(".grin_header_for_output");
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	util::init_test_logger();
	{
		clean_output_dir(".grin_header_for_output");
		let chain = init_chain(
			".grin_header_for_output",
			pow::mine_genesis_block().unwrap(),
		);
		let keychain = ExtKeychain::from_random_seed(false).unwrap();
		let mut reward_outputs = vec![];

		for n in 1..15 {
			let prev = chain.head_header().unwrap();
			let next_header_info =
				consensus::next_difficulty(prev.height + 1, chain.difficulty_iter().unwrap());
			let pk = ExtKeychainPath::new(1, n as u32, 0, 0, 0).to_identifier();
			let reward = libtx::reward::output(
				&keychain,
				&libtx::ProofBuilder::new(&keychain),
				&pk,
				0,
				false,
			)
			.unwrap();
			reward_outputs.push(reward.0.clone());
			let mut b =
				core::core::Block::new(&prev, &[], next_header_info.clone().difficulty, reward)
					.unwrap();
			b.header.timestamp = prev.timestamp + Duration::seconds(60);
			b.header.pow.secondary_scaling = next_header_info.secondary_scaling;

			chain.set_txhashset_roots(&mut b).unwrap();

			let edge_bits = if n == 2 {
				global::min_edge_bits() + 1
			} else {
				global::min_edge_bits()
			};
			b.header.pow.proof.edge_bits = edge_bits;
			pow::pow_size(
				&mut b.header,
				next_header_info.difficulty,
				global::proofsize(),
				edge_bits,
			)
			.unwrap();
			b.header.pow.proof.edge_bits = edge_bits;

			chain.process_block(b, chain::Options::MINE).unwrap();

			let header_for_output = chain
				.get_header_for_output(reward_outputs[n - 1].commitment())
				.unwrap();
			assert_eq!(header_for_output.height, n as u64);

			chain.validate(false).unwrap();
		}

		// Check all output positions are as expected
		for n in 1..15 {
			let header_for_output = chain
				.get_header_for_output(reward_outputs[n - 1].commitment())
				.unwrap();
			assert_eq!(header_for_output.height, n as u64);
		}
	}
	// Cleanup chain directory
	clean_output_dir(".grin_header_for_output");
}

/// Build a negative output. This function must not be used outside of tests.
/// The commitment will be an inversion of the value passed in and the value is
/// subtracted from the sum.
fn build_output_negative<K, B>(value: u64, key_id: Identifier) -> Box<Append<K, B>>
where
	K: Keychain,
	B: ProofBuild,
{
	Box::new(
		move |build, acc| -> Result<(Transaction, BlindSum), Error> {
			let (tx, sum) = acc?;

			// TODO: proper support for different switch commitment schemes
			let switch = SwitchCommitmentType::Regular;

			let commit = build.keychain.commit(value, &key_id, switch)?;

			// invert commitment
			let commit = build.keychain.secp().commit_sum(vec![], vec![commit])?;

			eprintln!("Building output: {}, {:?}", value, commit);

			// build a proof with a rangeproof of 0 as a placeholder
			// the test will replace this later
			let proof = proof::create(
				build.keychain,
				build.builder,
				0,
				&key_id,
				switch,
				commit,
				None,
			)?;

			// we return the output and the value is subtracted instead of added
			Ok((
				tx.with_output(Output::new(OutputFeatures::Plain, commit, proof)),
				sum.sub_key_id(key_id.to_value_path(value)),
			))
		},
	)
}

/// Test the duplicate rangeproof bug
#[test]
fn test_overflow_cached_rangeproof() {
	clean_output_dir(".grin_overflow");
	global::set_local_chain_type(ChainTypes::AutomatedTesting);

	util::init_test_logger();
	{
		let chain = init_chain(".grin_overflow", pow::mine_genesis_block().unwrap());
		let prev = chain.head_header().unwrap();
		let kc = ExtKeychain::from_random_seed(false).unwrap();
		let pb = ProofBuilder::new(&kc);

		let mut head = prev;

		// mine the first block and keep track of the block_hash
		// so we can spend the coinbase later
		let b = prepare_block(&kc, &head, &chain, 2);

		assert!(b.outputs()[0].is_coinbase());
		head = b.header.clone();
		chain
			.process_block(b.clone(), chain::Options::SKIP_POW)
			.unwrap();

		// now mine three further blocks
		for n in 3..6 {
			let b = prepare_block(&kc, &head, &chain, n);
			head = b.header.clone();
			chain.process_block(b, chain::Options::SKIP_POW).unwrap();
		}

		// create a few keys for use in txns
		let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
		let key_id30 = ExtKeychainPath::new(1, 30, 0, 0, 0).to_identifier();
		let key_id31 = ExtKeychainPath::new(1, 31, 0, 0, 0).to_identifier();
		let key_id32 = ExtKeychainPath::new(1, 32, 0, 0, 0).to_identifier();

		// build a regular transaction so we have a rangeproof to copy
		let tx1 = build::transaction(
			KernelFeatures::Plain { fee: 20000.into() },
			&[
				build::coinbase_input(consensus::REWARD, key_id2.clone()),
				build::output(consensus::REWARD - 20000, key_id30.clone()),
			],
			&kc,
			&pb,
		)
		.unwrap();

		// mine block with tx1
		let next = prepare_block_tx(&kc, &head, &chain, 7, &[tx1.clone()]);
		let prev_main = next.header.clone();
		chain
			.process_block(next.clone(), chain::Options::SKIP_POW)
			.unwrap();
		chain.validate(false).unwrap();

		// create a second tx that contains a negative output
		// and a positive output for 1m grin
		let mut tx2 = build::transaction(
			KernelFeatures::Plain { fee: 0.into() },
			&[
				build::input(consensus::REWARD - 20000, key_id30.clone()),
				build::output(
					consensus::REWARD - 20000 + 1_000_000_000_000_000,
					key_id31.clone(),
				),
				build_output_negative(1_000_000_000_000_000, key_id32.clone()),
			],
			&kc,
			&pb,
		)
		.unwrap();

		// make sure tx1 only has one output as expected
		assert_eq!(tx1.body.outputs.len(), 1);
		let last_rp = tx1.body.outputs[0].proof;

		// overwrite all our rangeproofs with the rangeproof from last block
		for i in 0..tx2.body.outputs.len() {
			tx2.body.outputs[i].proof = last_rp;
		}

		let next = prepare_block_tx(&kc, &prev_main, &chain, 8, &[tx2.clone()]);
		// process_block fails with verifier_cache disabled or with correct verifier_cache
		// implementations
		let res = chain.process_block(next, chain::Options::SKIP_POW);

		assert_eq!(
			res.unwrap_err(),
			chain::Error::InvalidBlockProof {
				source: block::Error::Transaction(transaction::Error::Secp(
					util::secp::Error::InvalidRangeProof
				))
			}
		);
	}
	clean_output_dir(".grin_overflow");
}

// Use diff as both diff *and* key_idx for convenience (deterministic private key for test blocks)
fn prepare_block<K>(kc: &K, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block
where
	K: Keychain,
{
	let key_idx = diff as u32;
	prepare_block_key_idx(kc, prev, chain, diff, key_idx)
}

fn prepare_block_key_idx<K>(
	kc: &K,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	key_idx: u32,
) -> Block
where
	K: Keychain,
{
	let mut b = prepare_block_nosum(kc, prev, diff, key_idx, &[]);
	chain.set_txhashset_roots(&mut b).unwrap();
	b
}

// Use diff as both diff *and* key_idx for convenience (deterministic private key for test blocks)
fn prepare_block_tx<K>(
	kc: &K,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	txs: &[Transaction],
) -> Block
where
	K: Keychain,
{
	let key_idx = diff as u32;
	prepare_block_tx_key_idx(kc, prev, chain, diff, key_idx, txs)
}

fn prepare_block_tx_key_idx<K>(
	kc: &K,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	key_idx: u32,
	txs: &[Transaction],
) -> Block
where
	K: Keychain,
{
	let mut b = prepare_block_nosum(kc, prev, diff, key_idx, txs);
	chain.set_txhashset_roots(&mut b).unwrap();
	b
}

fn prepare_block_nosum<K>(
	kc: &K,
	prev: &BlockHeader,
	diff: u64,
	key_idx: u32,
	txs: &[Transaction],
) -> Block
where
	K: Keychain,
{
	let proof_size = global::proofsize();
	let key_id = ExtKeychainPath::new(1, key_idx, 0, 0, 0).to_identifier();

	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let reward =
		libtx::reward::output(kc, &libtx::ProofBuilder::new(kc), &key_id, fees, false).unwrap();
	let mut b = match core::core::Block::new(prev, txs, Difficulty::from_num(diff), reward) {
		Err(e) => panic!("{:?}", e),
		Ok(b) => b,
	};
	b.header.timestamp = prev.timestamp + Duration::seconds(60);
	b.header.pow.total_difficulty = prev.total_difficulty() + Difficulty::from_num(diff);
	b.header.pow.proof = pow::Proof::random(proof_size);
	b
}

#[test]
#[ignore]
fn actual_diff_iter_output() {
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	let genesis_block = pow::mine_genesis_block().unwrap();
	let chain = chain::Chain::init(
		"../.grin".to_string(),
		Arc::new(NoopAdapter {}),
		genesis_block,
		pow::verify_size,
		false,
	)
	.unwrap();
	let iter = chain.difficulty_iter().unwrap();
	let mut last_time = 0;
	let mut first = true;
	for elem in iter.into_iter() {
		if first {
			last_time = elem.timestamp;
			first = false;
		}
		println!(
			"next_difficulty time: {}, diff: {}, duration: {} ",
			elem.timestamp,
			elem.difficulty.to_num(),
			last_time - elem.timestamp
		);
		last_time = elem.timestamp;
	}
}
