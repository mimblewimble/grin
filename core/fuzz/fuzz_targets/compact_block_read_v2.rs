#![no_main]
use libfuzzer_sys::fuzz_target;

extern crate grin_core;

use grin_core::core::UntrustedCompactBlock;
use grin_core::global;
use grin_core::ser::{self, DeserializationMode};

fuzz_target!(|data: &[u8]| {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	let mut d = data.clone();
	let _t: Result<UntrustedCompactBlock, ser::Error> =
		ser::deserialize(&mut d, ser::ProtocolVersion(2), DeserializationMode::Full);
});
