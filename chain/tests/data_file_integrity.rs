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

use self::core::genesis;
use grin_core as core;
use grin_util as util;

mod chain_test_helper;

use self::chain_test_helper::{clean_output_dir, init_chain, mine_chain};

#[test]
fn data_files() {
	util::init_test_logger();

	let chain_dir = ".grin_df";
	clean_output_dir(chain_dir);

	// Mine a few blocks on a new chain.
	{
		let chain = mine_chain(chain_dir, 4);
		chain.validate(false).unwrap();
		assert_eq!(chain.head().unwrap().height, 3);
	};

	// Now reload the chain from existing data files and check it is valid.
	{
		let chain = init_chain(chain_dir, genesis::genesis_dev());
		chain.validate(false).unwrap();
		assert_eq!(chain.head().unwrap().height, 3);
	}

	// Cleanup chain directory
	clean_output_dir(chain_dir);
}
