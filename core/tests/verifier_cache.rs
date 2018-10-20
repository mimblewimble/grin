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

extern crate chrono;
extern crate grin_core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

use std::sync::Arc;
use util::RwLock;

pub mod common;

use grin_core::core::verifier_cache::{LruVerifierCache, VerifierCache};
use grin_core::core::{Output, OutputFeatures};
use keychain::{ExtKeychain, Keychain};
use wallet::libtx::proof;

fn verifier_cache() -> Arc<RwLock<VerifierCache>> {
	Arc::new(RwLock::new(LruVerifierCache::new()))
}

#[test]
fn test_verifier_cache_rangeproofs() {
	let cache = verifier_cache();

	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let commit = keychain.commit(5, &key_id).unwrap();
	let proof = proof::create(&keychain, 5, &key_id, commit, None).unwrap();

	let out = Output {
		features: OutputFeatures::DEFAULT_OUTPUT,
		commit: commit,
		proof: proof,
	};

	// Check our output is not verified according to the cache.
	{
		let mut cache = cache.write();
		let unverified = cache.filter_rangeproof_unverified(&vec![out]);
		assert_eq!(unverified, vec![out]);
	}

	// Add our output to the cache.
	{
		let mut cache = cache.write();
		cache.add_rangeproof_verified(vec![out]);
	}

	// Check it shows as verified according to the cache.
	{
		let mut cache = cache.write();
		let unverified = cache.filter_rangeproof_unverified(&vec![out]);
		assert_eq!(unverified, vec![]);
	}
}
