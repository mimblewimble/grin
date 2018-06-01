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

//! Core types

pub mod block;
pub mod hash;
pub mod id;
pub mod pmmr;
pub mod target;
pub mod transaction;
use consensus::GRIN_BASE;
#[allow(dead_code)]
use rand::{thread_rng, Rng};
use std::cmp::Ordering;
use std::num::ParseFloatError;
use std::{fmt, iter};

use util::secp::pedersen::*;
use util::{secp, secp_static, static_secp_instance};

pub use self::block::*;
pub use self::id::ShortId;
pub use self::transaction::*;
use core::hash::Hashed;
use global;
use keychain;
use keychain::BlindingFactor;
use ser::{Error, Readable, Reader, Writeable, Writer};

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
	) -> Result<(Commitment, Commitment), keychain::Error> {
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
	) -> Result<Commitment, secp::Error> {
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
			secp.commit_sum(output_commits, input_commits)
		}
	}

	/// Vector of input commitments to verify.
	fn inputs_committed(&self) -> Vec<Commitment>;

	/// Vector of output commitments to verify.
	fn outputs_committed(&self) -> Vec<Commitment>;

	/// Vector of kernel excesses to verify.
	fn kernels_committed(&self) -> Vec<Commitment>;
}

/// Proof of work
#[derive(Clone, PartialOrd, PartialEq)]
pub struct Proof {
	/// The nonces
	pub nonces: Vec<u32>,
}

impl fmt::Debug for Proof {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Cuckoo(")?;
		for (i, val) in self.nonces[..].iter().enumerate() {
			write!(f, "{:x}", val)?;
			if i < self.nonces.len() - 1 {
				write!(f, " ")?;
			}
		}
		write!(f, ")")
	}
}

impl Eq for Proof {}

impl Proof {
	/// Builds a proof with all bytes zeroed out
	pub fn new(in_nonces: Vec<u32>) -> Proof {
		Proof { nonces: in_nonces }
	}

	/// Builds a proof with all bytes zeroed out
	pub fn zero(proof_size: usize) -> Proof {
		Proof {
			nonces: vec![0; proof_size],
		}
	}

	/// Builds a proof with random POW data,
	/// needed so that tests that ignore POW
	/// don't fail due to duplicate hashes
	pub fn random(proof_size: usize) -> Proof {
		let mut rng = thread_rng();
		let v: Vec<u32> = iter::repeat(())
			.map(|()| rng.gen())
			.take(proof_size)
			.collect();
		Proof { nonces: v }
	}

	/// Converts the proof to a vector of u64s
	pub fn to_u64s(&self) -> Vec<u64> {
		let mut out_nonces = Vec::with_capacity(self.proof_size());
		for n in &self.nonces {
			out_nonces.push(*n as u64);
		}
		out_nonces
	}

	/// Converts the proof to a vector of u32s
	pub fn to_u32s(&self) -> Vec<u32> {
		self.clone().nonces
	}

	/// Converts the proof to a proof-of-work Target so they can be compared.
	/// Hashes the Cuckoo Proof data.
	pub fn to_difficulty(&self) -> target::Difficulty {
		target::Difficulty::from_hash(&self.hash())
	}

	/// Returns the proof size
	pub fn proof_size(&self) -> usize {
		self.nonces.len()
	}
}

impl Readable for Proof {
	fn read(reader: &mut Reader) -> Result<Proof, Error> {
		let proof_size = global::proofsize();
		let mut pow = vec![0u32; proof_size];
		for n in 0..proof_size {
			pow[n] = reader.read_u32()?;
		}
		Ok(Proof::new(pow))
	}
}

impl Writeable for Proof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		for n in 0..self.proof_size() {
			writer.write_u32(self.nonces[n])?;
		}
		Ok(())
	}
}

/// Common method for parsing an amount from human-readable, and converting
/// to internally-compatible u64

pub fn amount_from_hr_string(amount: &str) -> Result<u64, ParseFloatError> {
	let amount = amount.parse::<f64>()?;
	Ok((amount * GRIN_BASE as f64) as u64)
}

/// Common method for converting an amount to a human-readable string

pub fn amount_to_hr_string(amount: u64) -> String {
	let amount = (amount as f64 / GRIN_BASE as f64) as f64;
	let places = (GRIN_BASE as f64).log(10.0) as usize + 1;
	format!("{:.*}", places, amount)
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	pub fn test_amount_to_hr() {
		assert!(50123456789 == amount_from_hr_string("50.123456789").unwrap());
		assert!(50 == amount_from_hr_string(".000000050").unwrap());
		assert!(1 == amount_from_hr_string(".000000001").unwrap());
		assert!(0 == amount_from_hr_string(".0000000009").unwrap());
		assert!(500_000_000_000 == amount_from_hr_string("500").unwrap());
		assert!(
			5_000_000_000_000_000_000 == amount_from_hr_string("5000000000.00000000000").unwrap()
		);
	}

	#[test]
	pub fn test_hr_to_amount() {
		assert!("50.123456789" == amount_to_hr_string(50123456789));
		assert!("0.000000050" == amount_to_hr_string(50));
		assert!("0.000000001" == amount_to_hr_string(1));
		assert!("500.000000000" == amount_to_hr_string(500_000_000_000));
		assert!("5000000000.000000000" == amount_to_hr_string(5_000_000_000_000_000_000));
	}

}
