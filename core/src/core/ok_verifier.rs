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

//! OKVerifier trait for batch verifying outputs and kernels.
//! We pass a "caching verifier" into the block validation processing with this.

use lru_cache::LruCache;

use core::hash::{Hash, Hashed};
use core::{Output, TxKernel};
use util::secp;
use util::secp::pedersen::{Commitment, RangeProof};
use util::LOGGER;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Error {
	Rangeproof,
	KernelSignature,
	Secp(secp::Error),
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

pub trait OKVerifier {
	fn verify_rangeproofs(&mut self, items: &Vec<Output>) -> Result<(), Error>;

	fn verify_kernel_signatures(&mut self, items: &Vec<TxKernel>) -> Result<(), Error>;
}

pub struct SimpleOKVerifier {}

impl SimpleOKVerifier {
	pub fn new() -> SimpleOKVerifier {
		SimpleOKVerifier {}
	}
}

impl OKVerifier for SimpleOKVerifier {
	fn verify_rangeproofs(&mut self, items: &Vec<Output>) -> Result<(), Error> {
		warn!(
			LOGGER,
			"simple_ok_verifier: verify_rangeproofs: {}",
			items.len()
		);

		let mut commits: Vec<Commitment> = vec![];
		let mut proofs: Vec<RangeProof> = vec![];

		if items.len() == 0 {
			return Ok(());
		}

		// unfortunately these have to be aligned in memory for the underlying
		// libsecp call
		for x in items {
			commits.push(x.commit.clone());
			proofs.push(x.proof.clone());
		}

		Output::batch_verify_proofs(&commits, &proofs)?;
		Ok(())
	}

	fn verify_kernel_signatures(&mut self, items: &Vec<TxKernel>) -> Result<(), Error> {
		warn!(
			LOGGER,
			"simple_ok_verifier: verify_kernel_signatures: {}",
			items.len()
		);

		for x in items {
			x.verify()?;
		}
		Ok(())
	}
}

pub struct CachingOKVerifier {
	verified_rangeproof_cache: LruCache<Hash, bool>,
	verified_kernel_sig_cache: LruCache<Hash, bool>,
}

impl CachingOKVerifier {
	/// TODO how big should these caches be?
	/// They need to be *at least* large enough to cover a maxed out block.
	pub fn new() -> CachingOKVerifier {
		CachingOKVerifier {
			verified_rangeproof_cache: LruCache::new(10_000),
			verified_kernel_sig_cache: LruCache::new(10_000),
		}
	}
}

impl OKVerifier for CachingOKVerifier {
	fn verify_rangeproofs(&mut self, items: &Vec<Output>) -> Result<(), Error> {
		warn!(
			LOGGER,
			"caching_ok_verifier: verify_rangeproofs: {}",
			items.len()
		);

		// Just return immediately if we have nothing to verify.
		if items.len() == 0 {
			return Ok(());
		}

		let mut commits: Vec<Commitment> = vec![];
		let mut proofs: Vec<RangeProof> = vec![];
		let mut proof_keys: Vec<Hash> = vec![];

		for x in items {
			// Note: cache key here is the hash of the rangeproof itself (not the output).
			let key = x.proof.hash();

			if self.verified_rangeproof_cache.contains_key(&key) {
				warn!(LOGGER, "caching_ok_verifier: rangeproof cache hit",);
			} else {
				warn!(LOGGER, "caching_ok_verifier: rangeproof cache miss",);
				commits.push(x.commit.clone());
				proofs.push(x.proof.clone());
				proof_keys.push(key.clone());
			}
		}

		if proofs.is_empty() {
			Ok(())
		} else {
			let res = Output::batch_verify_proofs(&commits, &proofs);
			if let Err(e) = res {
				return Err(Error::Secp(e));
			} else {
				for key in proof_keys {
					self.verified_rangeproof_cache.insert(key, true);
				}
				Ok(())
			}
		}
	}

	fn verify_kernel_signatures(&mut self, items: &Vec<TxKernel>) -> Result<(), Error> {
		warn!(
			LOGGER,
			"caching_ok_verifier: verify_kernel_signatures: {}",
			items.len()
		);

		// Just return immediately if we have nothing to verify.
		if items.len() == 0 {
			return Ok(());
		}

		for x in items {
			let key = x.hash();
			if self.verified_kernel_sig_cache.contains_key(&key) {
				warn!(LOGGER, "caching_ok_verifier: kernel sig cache hit",);
			} else {
				warn!(LOGGER, "caching_ok_verifier: kernel sig cache miss",);
				let res = x.verify();
				if let Err(e) = res {
					return Err(Error::Secp(e));
				} else {
					self.verified_kernel_sig_cache.insert(key, true);
				}
			}
		}
		Ok(())
	}
}

pub struct DeserializationOKVerifier {}

impl DeserializationOKVerifier {
	pub fn new() -> DeserializationOKVerifier {
		DeserializationOKVerifier {}
	}
}

impl OKVerifier for DeserializationOKVerifier {
	fn verify_rangeproofs(&mut self, items: &Vec<Output>) -> Result<(), Error> {
		// no-op - we skip rangeproof verification during deserialization.
		warn!(LOGGER, "verify_rangeproofs: skipped during deserialization");
		Ok(())
	}

	fn verify_kernel_signatures(&mut self, items: &Vec<TxKernel>) -> Result<(), Error> {
		// no-op - we skip kernel signature verification during deserialization.
		warn!(
			LOGGER,
			"verify_kernel_signatures: skipped during deserialization"
		);
		Ok(())
	}
}
