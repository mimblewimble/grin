#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate grin_core;
extern crate lazy_static;

use grin_core::ser;
use grin_core::core::transaction;

lazy_static!{
}

fuzz_target!(|data: &[u8]| {
	let mut d = data.clone();
	let _t: Result<transaction::Transaction, ser::Error> = ser::deserialize(&mut d);
});
