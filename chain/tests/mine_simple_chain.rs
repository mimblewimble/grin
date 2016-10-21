extern crate grin_core;
extern crate grin_chain;
extern crate rand;
extern crate secp256k1zkp as secp;

use rand::os::OsRng;

use grin_chain::types::*;
use grin_core::pow;
use grin_core::core;

#[test]
fn mine_empty_chain() {
  let curve = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	let mut rng = OsRng::new().unwrap();
	let store = grin_chain::store::ChainKVStore::new().unwrap();

  // save a genesis block
  let gen = grin_core::genesis::genesis(); 
  assert!(store.save_block(&gen).is_none());

  // setup a new head tip
  let tip = Tip::new(gen.hash());
  assert!(store.save_head(&tip).is_none());

  // mine and add a few blocks
  let mut prev = gen;
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	let reward_key = secp::key::SecretKey::new(&secp, &mut rng);

  for n in 1..6 {
    let mut b = core::Block::new(prev.header, vec![], reward_key).unwrap();
    println!("=> {} {:?}", b.header.height, b.verify(&curve));

    let (proof, nonce) = pow::pow20(&b, core::Proof(pow::MAX_TARGET)).unwrap();
    b.header.pow = proof;
    b.header.nonce = nonce;
    if let Some(e) = grin_chain::pipe::process_block(&b, &store, grin_chain::pipe::EASY_POW) {
      println!("err: {:?}", e);
      panic!();
    }

    // checking our new head
    let head = store.head().unwrap();
    assert_eq!(head.height, n);
    assert_eq!(head.last_block_h, b.hash());

    prev = b;
  }
}
