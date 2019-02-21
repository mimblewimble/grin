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

use cuckoocache::cache::{Cache, SaltedHasher};

use crate::core::hash::{DefaultHashable, Hash, Hashed, ZERO_HASH};
use crate::core::{Output, TxKernel};
use crate::util::RwLock;
use rand::prelude::random;

struct VerifierHasher {
	salt: Hash,
}
impl<E: DefaultHashable> SaltedHasher<E> for VerifierHasher {
	fn new() -> Self {
		Self {
			salt: Hash::from_vec(&random::<[u8; 32]>()[..]),
		}
	}
	fn hashes(&self, e: &E) -> [u32; 8] {
		let mut hs = [0u32; 8];

		for (v, h) in (self.salt, e)
			.hash()
			.as_bytes()
			.chunks_exact(4)
			.zip(hs.iter_mut())
		{
			let mut u = [0u8; 4];
			u.copy_from_slice(v);
			*h = u32::from_ne_bytes(u);
		}
		hs
	}
}
impl Default for VerifierHasher {
	fn default() -> Self {
		Self { salt: ZERO_HASH }
	}
}
/// Verifier cache for caching expensive verification results.
/// Specifically the following -
///   * kernel signature verification
///   * output rangeproof verification
pub trait VerifierCache: Sync + Send {
	/// Takes a vec of tx kernels and returns those kernels
	/// that have not yet been verified.
	fn filter_kernel_sig_unverified(&self, kernels: &[TxKernel]) -> Vec<TxKernel>;
	/// Takes a vec of tx outputs and returns those outputs
	/// that have not yet had their rangeproofs verified.
	fn filter_rangeproof_unverified(&self, outputs: &[Output]) -> Vec<Output>;
	/// Adds a vec of tx kernels to the cache (used in conjunction with the the filter above).
	fn add_kernel_sig_verified(&self, kernels: Vec<TxKernel>);
	/// Adds a vec of outputs to the cache (used in conjunction with the the filter above).
	fn add_rangeproof_verified(&self, outputs: Vec<Output>);
}

/// An implementation of verifier_cache using lru_cache.
/// Caches tx kernels by kernel hash.
/// Caches outputs by output rangeproof hash (rangeproofs are committed to separately).
pub struct LruVerifierCache {
	kernel_sig_verification_cache: RwLock<Cache<TxKernel, VerifierHasher>>,
	rangeproof_verification_cache:
		RwLock<Cache<crate::util::secp::pedersen::RangeProof, VerifierHasher>>,
}

impl LruVerifierCache {
	/// TODO how big should these caches be?
	/// They need to be *at least* large enough to cover a maxed out block.
	/// Because we're using a low overhead cache, use 2x the entries
	pub fn new() -> LruVerifierCache {
		let c = LruVerifierCache {
			kernel_sig_verification_cache: RwLock::new(Cache::empty()),
			rangeproof_verification_cache: RwLock::new(Cache::empty()),
		};
		c.kernel_sig_verification_cache.write().setup(2 * 50_000);
		c.rangeproof_verification_cache.write().setup(2 * 50_000);
		c
	}
}

impl VerifierCache for LruVerifierCache {
	fn filter_kernel_sig_unverified(&self, kernels: &[TxKernel]) -> Vec<TxKernel> {
		let cache = &self.kernel_sig_verification_cache;
		let res = kernels
			.iter()
			.filter(|x| !cache.read().contains(&x, false))
			.cloned()
			.collect::<Vec<_>>();
		trace!(
			"lru_verifier_cache: kernel sigs: {}, not cached (must verify): {}",
			kernels.len(),
			res.len()
		);
		res
	}

	fn filter_rangeproof_unverified(&self, outputs: &[Output]) -> Vec<Output> {
		let cache = &self.rangeproof_verification_cache.read();
		let res = outputs
			.iter()
			.filter(|x| !cache.contains(&x.proof, false))
			.cloned()
			.collect::<Vec<_>>();
		trace!(
			"lru_verifier_cache: rangeproofs: {}, not cached (must verify): {}",
			outputs.len(),
			res.len()
		);
		res
	}

	fn add_kernel_sig_verified(&self, kernels: Vec<TxKernel>) {
		let cache = &mut self.kernel_sig_verification_cache.write();
		for k in kernels {
			cache.insert(&k);
		}
	}

	fn add_rangeproof_verified(&self, outputs: Vec<Output>) {
		let cache = &mut self.rangeproof_verification_cache.write();
		for o in outputs {
			cache.insert(&o.proof);
		}
	}
}
