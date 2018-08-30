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

pub trait VerifierCache: Sync + Send {
	fn filter_kernel_sig_unverified(&mut self, kernels: &Vec<TxKernel>) -> Vec<TxKernel>;
	fn filter_rangeproof_unverified(&mut self, outputs: &Vec<Output>) -> Vec<Output>;
	fn add_kernel_sig_verified(&mut self, kernels: Vec<TxKernel>);
	fn add_rangeproof_verified(&mut self, outputs: Vec<Output>);
}

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
		kernels
			.into_iter()
			.filter(|x| !*self.kernel_sig_verification_cache.get_mut(&x.hash()).unwrap_or(&mut false))
			.cloned()
			.collect()
	}

	fn filter_rangeproof_unverified(&mut self, outputs: &Vec<Output>) -> Vec<Output> {
		outputs
			.into_iter()
			.filter(|x| !*self.rangeproof_verification_cache.get_mut(&x.proof.hash()).unwrap_or(&mut false))
			.cloned()
			.collect()
	}

	fn add_kernel_sig_verified(&mut self, kernels: Vec<TxKernel>) {
		for k in kernels {
		self.kernel_sig_verification_cache
			.insert(k.hash(), true);
		}
	}

	fn add_rangeproof_verified(&mut self, outputs: Vec<Output>) {
		for o in outputs {
			self.rangeproof_verification_cache
				.insert(o.proof.hash(), true);
		}
	}
}
