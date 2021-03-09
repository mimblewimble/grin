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

use self::core::core::hash::Hashed;
use grin_core as core;
use grin_util as util;

mod chain_test_helper;

use self::chain_test_helper::{clean_output_dir, mine_chain};

#[test]
fn test_store_indices() {
	util::init_test_logger();

	let chain_dir = ".grin_idx_1";
	clean_output_dir(chain_dir);

	let chain = mine_chain(chain_dir, 4);

	// Check head exists in the db.
	assert_eq!(chain.head().unwrap().height, 3);

	// Check the header exists in the db.
	assert_eq!(chain.head_header().unwrap().height, 3);

	// Check header_by_height index.
	let block_header = chain.get_header_by_height(3).unwrap();
	let block_hash = block_header.hash();
	assert_eq!(block_hash, chain.head().unwrap().last_block_h);

	{
		// Block exists in the db.
		assert_eq!(chain.get_block(&block_hash).unwrap().hash(), block_hash);

		// Check we have block_sums in the db.
		assert!(chain.get_block_sums(&block_hash).is_ok());

		{
			// Start a new batch and delete the block.
			let store = chain.store();
			let batch = store.batch().unwrap();
			assert!(batch.delete_block(&block_hash).is_ok());

			// Block is deleted within this batch.
			assert!(batch.get_block(&block_hash).is_err());
		}

		// Check the batch did not commit any changes to the store .
		assert!(chain.get_block(&block_hash).is_ok());
	}

	// Cleanup chain directory
	clean_output_dir(chain_dir);
}
