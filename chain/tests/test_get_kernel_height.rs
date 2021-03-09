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

use grin_util as util;

mod chain_test_helper;

use self::chain_test_helper::{clean_output_dir, mine_chain};
use util::secp::pedersen::Commitment;

#[test]
fn test_get_kernel_height() {
	let chain_dir = ".grin.get_kernel_height";
	clean_output_dir(chain_dir);
	let chain = mine_chain(chain_dir, 5);
	assert_eq!(chain.head().unwrap().height, 4);

	// check we can safely look for non-existent kernel with min_height=None, max_height=None
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), None, None)
			.unwrap()
	);

	// check we can safely look for non-existent kernel with min_height=1, max_height=1
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), Some(1), Some(1))
			.unwrap()
	);

	// check we can safely look for non-existent kernel with min_height=1, max_height=100
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), Some(1), Some(100))
			.unwrap()
	);

	// check we can safely look for non-existent kernel with min_height=100, max_height=100
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), Some(100), Some(100))
			.unwrap()
	);

	// check we can safely look for non-existent kernel with min_height=0, max_height=1
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), Some(0), Some(1))
			.unwrap()
	);

	// check we can safely look for non-existent kernel with min_height=0, max_height=100
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), Some(0), Some(100))
			.unwrap()
	);

	// check we can safely look for non-existent kernel with min_height=0, max_height=None
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), Some(0), None)
			.unwrap()
	);

	// check we can safely look for non-existent kernel with min_height=100, max_height=None
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), Some(100), None)
			.unwrap()
	);

	// check we can safely look for non-existent kernel with min_height=2, max_height=1
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), Some(2), Some(1))
			.unwrap()
	);

	// check we can safely look for non-existent kernel with min_height=100, max_height=99
	assert_eq!(
		None,
		chain
			.get_kernel_height(&Commitment::from_vec(vec![]), Some(100), Some(99))
			.unwrap()
	);

	clean_output_dir(chain_dir);
}
