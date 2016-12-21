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
extern crate rand;
extern crate secp256k1zkp as secp;

use std::sync::Arc;
use rand::os::OsRng;

use grin_chain::types::*;
use grin_core::pow;
use grin_core::core;
use grin_core::consensus;

#[test]
fn mine_empty_chain() {
	let mut rng = OsRng::new().unwrap();
	let store = grin_chain::store::ChainKVStore::new(".grin".to_string()).unwrap();

  // save a genesis block
  let gen = grin_core::genesis::genesis(); 
  store.save_block(&gen).unwrap();

  // setup a new head tip
  let tip = Tip::new(gen.hash());
  store.save_head(&tip).unwrap();

  // mine and add a few blocks
  let mut prev = gen;
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	let reward_key = secp::key::SecretKey::new(&secp, &mut rng);
  let arc_store = Arc::new(store);
  let adapter = Arc::new(NoopAdapter{});

  for n in 1..4 {
    let mut b = core::Block::new(&prev.header, vec![], reward_key).unwrap();

    let (diff_target, _) = consensus::next_target(b.header.timestamp.to_timespec().sec,
                                                      prev.header.timestamp.to_timespec().sec,
                                                      prev.header.target,
                                                      prev.header.cuckoo_len);

    let (proof, nonce) = pow::pow_size(&b, diff_target, 15).unwrap();
    b.header.pow = proof;
    b.header.nonce = nonce;
    b.header.target = diff_target;
    grin_chain::pipe::process_block(&b, arc_store.clone(), adapter.clone(), grin_chain::pipe::EASY_POW).unwrap();

    // checking our new head
    let head = arc_store.clone().head().unwrap();
    assert_eq!(head.height, n);
    assert_eq!(head.last_block_h, b.hash());

    prev = b;
  }
}
