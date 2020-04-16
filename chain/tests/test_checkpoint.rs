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
use grin_core::core::hash::Hashed;
use grin_core::genesis;
use grin_util as util;

#[test]
fn init_from_checkpoint() {
	let chain_dir = ".grin.checkpoint";
	util::init_test_logger();
	clean_output_dir(chain_dir);

	// mine some blocks and assert the checkpoint is *previous* block based on chain head.
	{
		let chain = mine_chain(chain_dir, 3);
		let head = chain.head().unwrap();
		let checkpoint = chain
			.txhashset()
			.read()
			.last_checkpoint()
			.map(|x| x.hash())
			.unwrap();
		assert_eq!(head.prev_block_h, checkpoint);
	}

	// re-init chain from disk, using the checkpoint written previously.
	{
		let chain = init_chain(chain_dir, genesis::genesis_dev());
		let head = chain.head().unwrap();
		let checkpoint = chain
			.txhashset()
			.read()
			.last_checkpoint()
			.map(|x| x.hash())
			.unwrap();
		assert_eq!(head.prev_block_h, checkpoint);
	}

	clean_output_dir(chain_dir);
}
