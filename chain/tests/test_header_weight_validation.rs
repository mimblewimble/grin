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

use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;

mod chain_test_helper;

use self::chain_test_helper::{clean_output_dir, mine_chain};
use crate::chain::{Chain, Error, Options};
use crate::core::{
	consensus,
	core::{block, Block},
	global,
	libtx::{reward, ProofBuilder},
	pow,
};
use crate::keychain::{ExtKeychain, ExtKeychainPath, Keychain};
use chrono::Duration;

fn build_block(chain: &Chain) -> Block {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let pk = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();

	let prev = chain.head_header().unwrap();
	let next_header_info = consensus::next_difficulty(1, chain.difficulty_iter().unwrap());
	let reward = reward::output(&keychain, &ProofBuilder::new(&keychain), &pk, 0, false).unwrap();
	let mut block = Block::new(&prev, &[], next_header_info.clone().difficulty, reward).unwrap();

	block.header.timestamp = prev.timestamp + Duration::seconds(60);
	block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

	chain.set_txhashset_roots(&mut block).unwrap();

	let edge_bits = global::min_edge_bits();
	block.header.pow.proof.edge_bits = edge_bits;
	pow::pow_size(
		&mut block.header,
		next_header_info.difficulty,
		global::proofsize(),
		edge_bits,
	)
	.unwrap();

	block
}

#[test]
fn test_header_weight_validation() {
	let chain_dir = ".grin.header_weight";
	clean_output_dir(chain_dir);
	let chain = mine_chain(chain_dir, 5);
	assert_eq!(chain.head().unwrap().height, 4);

	let block = build_block(&chain);
	let mut header = block.header;

	// Artificially set the output_mmr_size too large for a valid block.
	// Note: We will validate this even if just processing the header.
	header.output_mmr_size = 1_000;

	let res = chain.process_block_header(&header, Options::NONE);

	// Weight validation is done via transaction body and results in a slightly counter-intuitive tx error.
	assert_eq!(res, Err(Error::Block(block::Error::TooHeavy)));

	clean_output_dir(chain_dir);
}
