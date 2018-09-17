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

//! VerifierCache trait for batch verifying outputs and kernels.
//! We pass a "caching verifier" into the block validation processing with this.

use lru_cache::LruCache;

use core::hash::{Hash, Hashed};
use core::{Output, TxKernel};
use util::LOGGER;

/// Verifier cache for caching expensive verification results.
/// Specifically the following -
///   * kernel signature verification
///   * output rangeproof verification
pub trait VerifierCache: Sync + Send {
	/// Takes a vec of tx kernels and returns those kernels
	/// that have not yet been verified.
	fn filter_kernel_sig_unverified(&mut self, kernels: &Vec<TxKernel>) -> Vec<TxKernel>;
	/// Takes a vec of tx outputs and returns those outputs
	/// that have not yet had their rangeproofs verified.
	fn filter_rangeproof_unverified(&mut self, outputs: &Vec<Output>) -> Vec<Output>;
	/// Adds a vec of tx kernels to the cache (used in conjunction with the the filter above).
	fn add_kernel_sig_verified(&mut self, kernels: Vec<TxKernel>);
	/// Adds a vec of outputs to the cache (used in conjunction with the the filter above).
	fn add_rangeproof_verified(&mut self, outputs: Vec<Output>);
}

/// An implementation of verifier_cache using lru_cache.
/// Caches tx kernels by kernel hash.
/// Caches outputs by output rangeproof hash (rangeproofs are committed to separately).
pub struct LruVerifierCache {
	kernel_sig_verification_cache: LruCache<Hash, bool>,
	rangeproof_verification_cache: LruCache<Hash, bool>,
}

unsafe impl Sync for LruVerifierCache {}
unsafe impl Send for LruVerifierCache {}

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
	fn filter_kernel_sig_unverified(&mut self, kernels: &Vec<TxKernel>) -> Vec<TxKernel> {
		let res = kernels
			.into_iter()
			.filter(|x| {
				!*self
					.kernel_sig_verification_cache
					.get_mut(&x.hash())
					.unwrap_or(&mut false)
			}).cloned()
			.collect::<Vec<_>>();
		debug!(
			LOGGER,
			"lru_verifier_cache: kernel sigs: {}, not cached (must verify): {}",
			kernels.len(),
			res.len()
		);
		res
	}

	fn filter_rangeproof_unverified(&mut self, outputs: &Vec<Output>) -> Vec<Output> {
		let res = outputs
			.into_iter()
			.filter(|x| {
				!*self
					.rangeproof_verification_cache
					.get_mut(&x.proof.hash())
					.unwrap_or(&mut false)
			}).cloned()
			.collect::<Vec<_>>();
		debug!(
			LOGGER,
			"lru_verifier_cache: rangeproofs: {}, not cached (must verify): {}",
			outputs.len(),
			res.len()
		);
		res
	}

	fn add_kernel_sig_verified(&mut self, kernels: Vec<TxKernel>) {
		for k in kernels {
			self.kernel_sig_verification_cache.insert(k.hash(), true);
		}
	}

	fn add_rangeproof_verified(&mut self, outputs: Vec<Output>) {
		for o in outputs {
			self.rangeproof_verification_cache
				.insert(o.proof.hash(), true);
		}
	}
}
