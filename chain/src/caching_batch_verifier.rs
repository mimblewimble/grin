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

//! For "cache aware" batch verifying of rangeproofs and kernel signatures.

use core::core::{Output, TxKernel};
use core::core::batch_verifier::{self, BatchVerifier};
use util::secp::pedersen::{Commitment, RangeProof};


pub struct CachingBatchVerifier {}

impl CachingBatchVerifier {
	pub fn new() -> CachingBatchVerifier {
		CachingBatchVerifier{}
	}
}

impl BatchVerifier for CachingBatchVerifier {
	fn verify_rangeproofs(&self, items: &Vec<Output>) -> Result<(), batch_verifier::Error> {
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

	fn verify_kernel_signatures(
		&self,
		items: &Vec<TxKernel>,
	) -> Result<(), batch_verifier::Error> {
		for x in items {
			x.verify()?;
		}
		Ok(())
	}
}
