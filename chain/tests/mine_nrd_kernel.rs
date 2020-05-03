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

use self::chain_test_helper::{clean_output_dir, mine_chain};

#[test]
fn mine_block_with_nrd_kernel() {
	let chain_dir = ".grin.nrd_kernel";
	clean_output_dir(chain_dir);
	let chain = mine_chain(chain_dir, 9);
	assert_eq!(chain.head().unwrap().height, 8);

	// now mine a block containing a tx with an NRD kernel.

	clean_output_dir(chain_dir);
	panic!("wat");
}
