#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate grin_core;


use grin_core::ser;
use grin_core::core::transaction;

fuzz_target!(|data: &[u8]| {
	let mut d = data.clone();
	let _t: Result<transaction::Transaction, ser::Error> = ser::deserialize(&mut d);
});
