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

use crate::core::hash::HashWriter;
use crate::core::{Output, TxKernel};
use crate::ser::Writeable;
use crate::util::RwLock;
use byteorder::{BigEndian, ByteOrder};

struct VerifierHasher {
	hasher: HashWriter,
}
impl<E: Writeable> SaltedHasher<E> for VerifierHasher {
	fn new() -> Self {
		Self {
			hasher: HashWriter::random_keyed(),
		}
	}
	fn hashes(&self, e: &E) -> [u32; 8] {
		let mut hasher = self.hasher.clone();
		Writeable::write(e, &mut hasher).unwrap();
		let mut h = [0; 32];
		hasher.finalize(&mut h);
		let mut ret = [0u32; 8];
		for i in 0..8 {
			ret[i] = BigEndian::read_u32(&h[i * 4..(i + 1) * 4]);
		}
		ret
	}
}
impl Default for VerifierHasher {
	fn default() -> Self {
		Self {
			hasher: HashWriter::default(),
		}
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
