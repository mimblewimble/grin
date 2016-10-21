//! Main crate putting together all the other crates that compose Grin into a
//! binary.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

extern crate grin_core as core;
extern crate grin_store as store;

use store::Store;
use core::genesis::genesis;

fn main() {
	let gen = genesis();
	let db = Store::open("./store").unwrap();
	let mut key = "block:".to_string().into_bytes();
	let mut hash_vec = gen.hash().to_vec();
	key.append(&mut hash_vec);
	db.put_ser(&key[..], &gen);
}
