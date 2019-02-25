#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate grin_core;
extern crate grin_p2p;

use grin_core::ser;
use grin_p2p::types::PeerAddr;

fuzz_target!(|data: &[u8]| {
	let mut d = data.clone();
	let _t: Result<PeerAddr, ser::Error> = ser::deserialize(&mut d);
});
