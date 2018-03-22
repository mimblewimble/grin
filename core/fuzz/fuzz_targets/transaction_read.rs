#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate grin_core;

use std::path::Path;
use std::fs::File;
use std::io::prelude::*;
use grin_core::ser;
use grin_core::core::{self, transaction};

fuzz_target!(|data: &[u8]| {
	let mut d = data.clone();
	let _t: Result<transaction::Transaction, ser::Error> = ser::deserialize(&mut d);
});
