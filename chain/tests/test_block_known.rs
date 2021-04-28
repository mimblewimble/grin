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

mod chain_test_helper;
use self::chain_test_helper::{clean_output_dir, init_chain, mine_chain};
use chain::Error;
use chain::Tip;
use grin_chain as chain;
use grin_core::core::hash::Hashed;
use grin_util as util;

#[test]
fn check_known() {
	let chain_dir = ".grin.check_known";
	util::init_test_logger();
	clean_output_dir(chain_dir);

	// mine some blocks
	let (latest, genesis) = {
		let chain = mine_chain(chain_dir, 3);
		let genesis = chain
			.get_block(&chain.get_header_by_height(0).unwrap().hash())
			.unwrap();
		let head = chain.head().unwrap();
		let latest = chain.get_block(&head.last_block_h).unwrap();
		(latest, genesis)
	};

	// attempt to reprocess latest block
	{
		let chain = init_chain(chain_dir, genesis.clone());
		let res = chain.process_block(latest.clone(), chain::Options::NONE);
		assert_eq!(
			res.unwrap_err(),
			Error::Unfit("duplicate block".to_string())
		);
	}

	// attempt to reprocess genesis block
	{
		let chain = init_chain(chain_dir, genesis.clone());
		let res = chain.process_block(genesis.clone(), chain::Options::NONE);
		assert_eq!(
			res.unwrap_err(),
			Error::Unfit("duplicate block".to_string())
		);
	}

	// reset chain head to earlier state
	{
		let chain = init_chain(chain_dir, genesis.clone());
		let store = chain.store();
		let batch = store.batch().unwrap();
		let head_header = chain.head_header().unwrap();
		let prev = batch.get_previous_header(&head_header).unwrap();
		batch.save_body_head(&Tip::from_header(&prev)).unwrap();
		batch.commit().unwrap();
	}

	// reprocess latest block and check the updated head
	{
		let chain = init_chain(chain_dir, genesis.clone());
		let head = chain
			.process_block(latest.clone(), chain::Options::NONE)
			.unwrap();
		assert_eq!(head, Some(Tip::from_header(&latest.header)));
	}

	clean_output_dir(chain_dir);
}
