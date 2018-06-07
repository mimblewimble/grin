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

//! The Committed trait and associated errors.

use keychain;
use keychain::BlindingFactor;

use util::secp::pedersen::*;
use util::{secp, secp_static, static_secp_instance};

/// Errors from summing and verifying kernel excesses via committed trait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
	/// Keychain related error.
	Keychain(keychain::Error),
	/// Secp related error.
	Secp(secp::Error),
	/// Kernel sums do not equal output sums.
	KernelSumMismatch,
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<keychain::Error> for Error {
	fn from(e: keychain::Error) -> Error {
		Error::Keychain(e)
	}
}

/// Implemented by types that hold inputs and outputs (and kernels)
/// containing Pedersen commitments.
/// Handles the collection of the commitments as well as their
/// summing, taking potential explicit overages of fees into account.
pub trait Committed {
	/// Gather the kernel excesses and sum them.
	fn sum_kernel_excesses(
		&self,
		offset: &BlindingFactor,
		extra_excess: Option<&Commitment>,
	) -> Result<(Commitment, Commitment), Error> {
		let zero_commit = secp_static::commit_to_zero_value();

		// then gather the kernel excess commitments
		let mut kernel_commits = self.kernels_committed();

		if let Some(extra) = extra_excess {
			kernel_commits.push(*extra);
		}

		// handle "zero commit" values by filtering them out here
		kernel_commits.retain(|x| *x != zero_commit);

		// sum the commitments
		let kernel_sum = {
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			secp.commit_sum(kernel_commits, vec![])?
		};

		// sum the commitments along with the
		// commit to zero built from the offset
		let kernel_sum_plus_offset = {
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			let mut commits = vec![kernel_sum];
			if *offset != BlindingFactor::zero() {
				let key = offset.secret_key(&secp)?;
				let offset_commit = secp.commit(0, key)?;
				commits.push(offset_commit);
			}
			secp.commit_sum(commits, vec![])?
		};

		Ok((kernel_sum, kernel_sum_plus_offset))
	}

	/// Gathers commitments and sum them.
	fn sum_commitments(
		&self,
		overage: i64,
		extra_commit: Option<&Commitment>,
	) -> Result<Commitment, Error> {
		let zero_commit = secp_static::commit_to_zero_value();

		// then gather the commitments
		let mut input_commits = self.inputs_committed();
		let mut output_commits = self.outputs_committed();

		// add the overage as output commitment if positive,
		// or as an input commitment if negative
		if overage != 0 {
			let over_commit = {
				let secp = static_secp_instance();
				let secp = secp.lock().unwrap();
				secp.commit_value(overage.abs() as u64).unwrap()
			};
			if overage < 0 {
				input_commits.push(over_commit);
			} else {
				output_commits.push(over_commit);
			}
		}

		if let Some(extra) = extra_commit {
			output_commits.push(*extra);
		}

		// handle "zero commit" values by filtering them out here
		output_commits.retain(|x| *x != zero_commit);
		input_commits.retain(|x| *x != zero_commit);

		// sum all that stuff
		{
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			let res = secp.commit_sum(output_commits, input_commits)?;
			Ok(res)
		}
	}

	/// Vector of input commitments to verify.
	fn inputs_committed(&self) -> Vec<Commitment>;

	/// Vector of output commitments to verify.
	fn outputs_committed(&self) -> Vec<Commitment>;

	/// Vector of kernel excesses to verify.
	fn kernels_committed(&self) -> Vec<Commitment>;

	/// Verify the sum of the kernel excesses equals the
	/// sum of the outputs, taking into account both
	/// the kernel_offset and overage.
	fn verify_kernel_sums(
		&self,
		overage: i64,
		kernel_offset: BlindingFactor,
		prev_output_sum: Option<&Commitment>,
		prev_kernel_sum: Option<&Commitment>,
	) -> Result<((Commitment, Commitment)), Error> {
		// Sum all input|output|overage commitments.
		let utxo_sum = self.sum_commitments(overage, prev_output_sum)?;

		// Sum the kernel excesses accounting for the kernel offset.
		let (kernel_sum, kernel_sum_plus_offset) =
			self.sum_kernel_excesses(&kernel_offset, prev_kernel_sum)?;

		if utxo_sum != kernel_sum_plus_offset {
			return Err(Error::KernelSumMismatch);
		}

		Ok((utxo_sum, kernel_sum))
	}
}
