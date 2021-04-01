// Copyright 2021 The Grin Developers
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

//! VerifierCache trait for batch verifying outputs and kernels.
//! We pass a "caching verifier" into the block validation processing with this.

use crate::core::hash::{Hash, Hashed};
use crate::core::{Output, TxKernel};
use lru_cache::LruCache;

/// Verifier cache for caching expensive verification results.
/// Specifically the following -
///   * kernel signature verification
///   * output rangeproof verification
pub trait VerifierCache: Sync + Send {
	/// Takes a vec of tx kernels and returns those kernels
	/// that have not yet been verified.
	fn filter_kernel_sig_unverified(&mut self, kernels: &[TxKernel]) -> Vec<TxKernel>;
	/// Takes a vec of tx outputs and returns those outputs
	/// that have not yet had their rangeproofs verified.
	fn filter_rangeproof_unverified(&mut self, outputs: &[Output]) -> Vec<Output>;
	/// Adds a vec of tx kernels to the cache (used in conjunction with the the filter above).
	fn add_kernel_sig_verified(&mut self, kernels: Vec<TxKernel>);
	/// Adds a vec of outputs to the cache (used in conjunction with the the filter above).
	fn add_rangeproof_verified(&mut self, outputs: Vec<Output>);
}

/// An implementation of verifier_cache using lru_cache.
/// Caches tx kernels by kernel hash.
/// Caches outputs by output rangeproof hash (rangeproofs are committed to separately).
pub struct LruVerifierCache {
	kernel_sig_verification_cache: LruCache<Hash, ()>,
	rangeproof_verification_cache: LruCache<Hash, ()>,
}

impl LruVerifierCache {
	/// TODO how big should these caches be?
	/// They need to be *at least* large enough to cover a maxed out block.
	pub fn new() -> LruVerifierCache {
		LruVerifierCache {
			kernel_sig_verification_cache: LruCache::new(50_000),
			rangeproof_verification_cache: LruCache::new(50_000),
		}
	}
}

impl VerifierCache for LruVerifierCache {
	fn filter_kernel_sig_unverified(&mut self, kernels: &[TxKernel]) -> Vec<TxKernel> {
		let res = kernels
			.iter()
			.filter(|x| !self.kernel_sig_verification_cache.contains_key(&x.hash()))
			.cloned()
			.collect::<Vec<_>>();
		trace!(
			"lru_verifier_cache: kernel sigs: {}, not cached (must verify): {}",
			kernels.len(),
			res.len()
		);
		res
	}

	fn filter_rangeproof_unverified(&mut self, outputs: &[Output]) -> Vec<Output> {
		let res = outputs
			.iter()
			.filter(|x| !self.rangeproof_verification_cache.contains_key(&x.hash()))
			.cloned()
			.collect::<Vec<_>>();
		trace!(
			"lru_verifier_cache: rangeproofs: {}, not cached (must verify): {}",
			outputs.len(),
			res.len()
		);
		res
	}

	fn add_kernel_sig_verified(&mut self, kernels: Vec<TxKernel>) {
		for k in kernels {
			self.kernel_sig_verification_cache.insert(k.hash(), ());
		}
	}

	fn add_rangeproof_verified(&mut self, outputs: Vec<Output>) {
		for o in outputs {
			self.rangeproof_verification_cache.insert(o.hash(), ());
		}
	}
}

#[cfg(test)]
mod test {
	#[test]
	fn test_verifier_cache() {
		use crate::core::transaction::KernelFeatures;
		use crate::core::transaction::Output;
		use crate::core::verifier_cache::LruVerifierCache;
		use crate::core::verifier_cache::VerifierCache;
		use crate::global;
		use crate::libtx::build;
		use crate::libtx::build::input;
		use crate::libtx::build::output;
		use crate::libtx::ProofBuilder;
		use keychain::{ExtKeychain, Keychain};

		// build some txns to use with cache.
		global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
		let keychain = ExtKeychain::from_random_seed(false).unwrap();
		let builder = ProofBuilder::new(&keychain);
		let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
		let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
		let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);
		let key_id4 = ExtKeychain::derive_key_id(1, 4, 0, 0, 0);

		let mut verifier_cache = LruVerifierCache::new();

		let tx = build::transaction(
			KernelFeatures::Plain { fee: 2.into() },
			&[input(7, key_id1), output(5, key_id2)],
			&keychain,
			&builder,
		)
		.unwrap();

		let tx2 = build::transaction(
			KernelFeatures::Plain { fee: 3.into() },
			&[input(9, key_id3), output(6, key_id4)],
			&keychain,
			&builder,
		)
		.unwrap();

		let output1 = tx.outputs()[0];
		let output2 = tx2.outputs()[0];
		let mixed_output = Output {
			identifier: output2.identifier,
			proof: output1.proof,
		};

		// add first output to verifier cache
		verifier_cache.add_rangeproof_verified(vec![output1]);

		// filter output2, should not be removed
		let outputs = verifier_cache.filter_rangeproof_unverified(&vec![output2]);
		assert_eq!(outputs[0], output2);
		assert_eq!(outputs.len(), 1);

		// filter mixed_output (would fail before the fix)
		let outputs = verifier_cache.filter_rangeproof_unverified(&vec![mixed_output]);
		assert_eq!(outputs[0], mixed_output);
		assert_eq!(outputs.len(), 1);

		// filter output 1, which has been verified
		let outputs = verifier_cache.filter_rangeproof_unverified(&vec![output1]);
		assert_eq!(outputs.len(), 0);
	}
}
