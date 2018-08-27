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
use util::secp;
use util::secp::pedersen::{Commitment, RangeProof};
use util::LOGGER;

pub trait VerifierCache: Sync + Send {
	fn is_kernel_sig_verified(&mut self, kernel: &TxKernel) -> bool;
	fn is_rangeproof_verified(&mut self, output: &Output) -> bool;
	fn add_kernel_sig_verified(&mut self, kernel: &TxKernel);
	fn add_rangeproof_verified(&mut self, output: &Output);
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
	fn is_kernel_sig_verified(&mut self, kernel: &TxKernel) -> bool {
		*self
			.kernel_sig_verification_cache
			.get_mut(&kernel.hash())
			.unwrap_or(&mut false)
	}

	fn is_rangeproof_verified(&mut self, output: &Output) -> bool {
		*self
			.rangeproof_verification_cache
			.get_mut(&output.proof.hash())
			.unwrap_or(&mut false)
	}

	fn add_kernel_sig_verified(&mut self, kernel: &TxKernel) {
		self.kernel_sig_verification_cache
			.insert(kernel.hash(), true);
	}

	fn add_rangeproof_verified(&mut self, output: &Output) {
		self.rangeproof_verification_cache
			.insert(output.proof.hash(), true);
	}
}
