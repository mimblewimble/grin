// Copyright 2016 The Grin Developers
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

extern crate grin_core;
extern crate grin_chain;
extern crate env_logger;
extern crate time;
extern crate rand;
extern crate secp256k1zkp as secp;

use std::sync::Arc;
use std::thread;
use rand::os::OsRng;

use grin_chain::types::*;
use grin_chain::store;
use grin_core::core::hash::Hashed;
use grin_core::core::target::Difficulty;
use grin_core::pow;
use grin_core::core;
use grin_core::consensus;

#[test]
fn mine_empty_chain() {
  env_logger::init();
	let mut rng = OsRng::new().unwrap();
  let chain = grin_chain::Chain::init(true, ".grin".to_string(), Arc::new(NoopAdapter{})).unwrap();

	// mine and add a few blocks
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	let reward_key = secp::key::SecretKey::new(&secp, &mut rng);

	for n in 1..4 {
    let prev = chain.head_header().unwrap();
		let mut b = core::Block::new(&prev, vec![], reward_key).unwrap();
		b.header.timestamp = prev.timestamp + time::Duration::seconds(60);

		let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
		b.header.difficulty = difficulty.clone();

		pow::pow_size(&mut b.header, difficulty, consensus::TEST_SIZESHIFT as u32).unwrap();
		chain.process_block(&b, grin_chain::EASY_POW).unwrap();

		// checking our new head
		let head = chain.head().unwrap();
		assert_eq!(head.height, n);
		assert_eq!(head.last_block_h, b.hash());
	}
}

#[test]
fn mine_forks() {
  env_logger::init();
	let mut rng = OsRng::new().unwrap();
  let chain = grin_chain::Chain::init(true, ".grin2".to_string(), Arc::new(NoopAdapter{})).unwrap();

	// mine and add a few blocks
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	let reward_key = secp::key::SecretKey::new(&secp, &mut rng);

	for n in 1..4 {
    let prev = chain.head_header().unwrap();
		let mut b = core::Block::new(&prev, vec![], reward_key).unwrap();
		b.header.timestamp = prev.timestamp + time::Duration::seconds(60);
    b.header.total_difficulty = Difficulty::from_num(2*n);
		chain.process_block(&b, grin_chain::SKIP_POW).unwrap();

		// checking our new head
    thread::sleep(::std::time::Duration::from_millis(50));
		let head = chain.head().unwrap();
		assert_eq!(head.height, n as u64);
		assert_eq!(head.last_block_h, b.hash());
		assert_eq!(head.prev_block_h, prev.hash());

    // build another block with higher difficulty
		let mut b = core::Block::new(&prev, vec![], reward_key).unwrap();
		b.header.timestamp = prev.timestamp + time::Duration::seconds(60);
    b.header.total_difficulty = Difficulty::from_num(2*n+1);
		chain.process_block(&b, grin_chain::SKIP_POW).unwrap();

		// checking head switch
    thread::sleep(::std::time::Duration::from_millis(50));
		let head = chain.head().unwrap();
		assert_eq!(head.height, n as u64);
		assert_eq!(head.last_block_h, b.hash());
		assert_eq!(head.prev_block_h, prev.hash());
	}
}
