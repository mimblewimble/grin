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

//! Lightweight readonly view into output MMR for convenience.

use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use util::secp::pedersen::{Commitment, RangeProof};

use core::core::committed::Committed;
use core::core::hash::{Hash, Hashed};
use core::core::merkle_proof::MerkleProof;
use core::core::pmmr::{self, PMMRReadonly, PMMR};
use core::core::{Block, BlockHeader, Input, Output, OutputFeatures, OutputIdentifier, TxKernel};

use error::{Error, ErrorKind};
use grin_store::pmmr::{PMMRBackend, PMMR_FILES};
use store::Batch;
use txhashset;
use txhashset::{input_pos_to_rewind, TxHashSet};

/// Readonly view of the UTXO set (based on output MMR).
pub struct UTXOView<'a> {
	pmmr: PMMRReadonly<'a, OutputIdentifier, PMMRBackend<OutputIdentifier>>,
	batch: &'a Batch<'a>,
}

impl<'a> UTXOView<'a> {
	pub fn new(
		pmmr: PMMRReadonly<'a, OutputIdentifier, PMMRBackend<OutputIdentifier>>,
		batch: &'a Batch,
	) -> UTXOView<'a> {
		UTXOView { pmmr, batch }
	}

	/// Validate a Committed (inputs & outputs) against the UTXO set.
	/// Every input must spend an output that currently exists in the UTXO set.
	pub fn validate(&self, committed: &Committed) -> Result<(), Error> {
		for out in committed.outputs_committed() {
			self.validate_output(out)?;
		}

		for input in committed.inputs_committed() {
			self.validate_input(input)?;
		}
		Ok(())
	}

	fn validate_input(&self, commit: Commitment) -> Result<(), Error> {
		let pos_res = self.batch.get_output_pos(&commit);
		if let Ok(pos) = pos_res {
			if let Some(_) = self.pmmr.get_data(pos) {
				return Ok(());
			}
		}
		Err(ErrorKind::AlreadySpent(commit).into())
	}

	fn validate_output(&self, commit: Commitment) -> Result<(), Error> {
		if let Ok(pos) = self.batch.get_output_pos(&commit) {
			if let Some(out_mmr) = self.pmmr.get_data(pos) {
				if out_mmr.commitment() == commit {
					return Err(ErrorKind::DuplicateCommitment(commit).into());
				}
			}
		}
		Ok(())
	}
}
