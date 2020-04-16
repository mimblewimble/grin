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

mod chain_test_helper;
use self::chain_test_helper::{clean_output_dir, init_chain, mine_chain};
use chain::Tip;
use grin_chain as chain;
use grin_core::core::hash::Hashed;
use grin_util as util;

#[test]
fn init_from_checkpoint() {
	let chain_dir = ".grin.checkpoint";
	util::init_test_logger();
	clean_output_dir(chain_dir);

	// mine some blocks
	// assert the checkpoint is *previous* block based on chain head
	let chain = mine_chain(chain_dir, 3);
	let genesis = chain
		.get_block(&chain.get_header_by_height(0).unwrap().hash())
		.unwrap();
	let head = chain.head().unwrap();
	let checkpoint = chain.txhashset().read().last_checkpoint().unwrap();
	assert_eq!(head.prev_block_h, checkpoint.hash());
	let head_orig = head.clone();

	// re-init chain from disk
	// using the checkpoint written previously
	let chain = init_chain(chain_dir, genesis.clone());
	let head = chain.head().unwrap();
	assert_eq!(head.prev_block_h, checkpoint.hash());
	assert_eq!(head_orig, head);

	// reset chain head to earlier state and forget about "latest" block
	// chain head now matches checkpoint
	let latest_block = chain.get_block(&head.last_block_h).unwrap();
	{
		let store = chain.store();
		let batch = store.batch().unwrap();
		batch
			.save_body_head(&Tip::from_header(&checkpoint))
			.unwrap();
		batch.delete_block(&latest_block.hash()).unwrap();
		batch.commit().unwrap();
	}

	// re-init chain from disk
	// assert the chain head corresponds to the checkpoint itself
	let chain = init_chain(chain_dir, genesis.clone());
	let head = chain.head().unwrap();
	assert_eq!(head.last_block_h, checkpoint.hash());

	// reprocess the "latest" block
	// assert we are back to "latest" at head
	let head = chain
		.process_block(latest_block, chain::Options::NONE)
		.unwrap()
		.unwrap();
	assert_eq!(head, head_orig);

	clean_output_dir(chain_dir);
}
