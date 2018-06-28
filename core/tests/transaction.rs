// Copyright 2018 The Grin Developers
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

//! Transaction integration tests
extern crate grin_core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

pub mod common;

use grin_core::core::{Output, OutputFeatures};
use grin_core::ser;
use keychain::{ExtKeychain, Keychain};
use util::secp;
use wallet::libtx::proof;

#[test]
fn test_output_ser_deser() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();
	let commit = keychain.commit(5, &key_id).unwrap();
	let msg = secp::pedersen::ProofMessage::empty();
	let proof = proof::create(&keychain, 5, &key_id, commit, None, msg).unwrap();

	let out = Output {
		features: OutputFeatures::DEFAULT_OUTPUT,
		commit: commit,
		proof: proof,
	};

	let mut vec = vec![];
	ser::serialize(&mut vec, &out).expect("serialized failed");
	let dout: Output = ser::deserialize(&mut &vec[..]).unwrap();

	assert_eq!(dout.features, OutputFeatures::DEFAULT_OUTPUT);
	assert_eq!(dout.commit, out.commit);
	assert_eq!(dout.proof, out.proof);
}
