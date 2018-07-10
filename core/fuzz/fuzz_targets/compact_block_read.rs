#![no_main]
extern crate grin_core;
#[macro_use]
extern crate libfuzzer_sys;

use grin_core::core::block;
use grin_core::ser;

fuzz_target!(|data: &[u8]| {
	let mut d = data.clone();
	let _t: Result<block::CompactBlock, ser::Error> = ser::deserialize(&mut d);
});
