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

use croaring::Bitmap;

use util::secp::pedersen::{Commitment, RangeProof};

use core::core::committed::Committed;
use core::core::hash::{Hash, Hashed};
use core::core::merkle_proof::MerkleProof;
use core::core::pmmr::{self, PMMR};
use core::core::{Block, BlockHeader, Input, Output, OutputFeatures, OutputIdentifier, TxKernel};

use error::{Error, ErrorKind};
use grin_store::pmmr::{PMMRBackend, PMMR_FILES};
use store::{Batch, ChainStore};
use txhashset;
use txhashset::{TxHashSet, PMMRHandle, input_pos_to_rewind};

pub struct UTXOView<'a> {
	output_pmmr: PMMR<'a, OutputIdentifier, PMMRBackend<OutputIdentifier>>,
	store: Arc<ChainStore>,
}

impl<'a> UTXOView<'a> {
	pub fn new(
		output_pmmr: &'a mut PMMRHandle<OutputIdentifier>,
		store: Arc<ChainStore>,
	) -> UTXOView<'a> {
		UTXOView {output_pmmr, store}
	}

	/// "Fast" validation of a set of inputs and outputs (from either a block or a transaction).
	/// This is a lightweight/faster alternative to something like apply_block().
	/// Inputs _must_ spend unspent outputs.
	/// Outputs _must not_ introduce duplicate commitments.
	pub fn validate_utxo_fast(
		&mut self,
		inputs: &Vec<Input>,
		outputs: &Vec<Output>,
	) -> Result<(), Error> {
		for out in outputs {
			self.validate_utxo_output(out)?;
		}

		for input in inputs {
			self.validate_utxo_input(input)?;
		}

		Ok(())
	}

	// TODO - Is this sufficient?
	fn validate_utxo_input(&mut self, input: &Input) -> Result<(), Error> {
		let commit = input.commitment();
		let pos_res = self.store.get_output_pos(&commit);
		if let Ok(pos) = pos_res {
			if let Some(_) = self.output_pmmr.get_data(pos) {
				return Ok(());
			}
		}
		Err(ErrorKind::AlreadySpent(commit).into())
	}

	/// TODO - Is this sufficient?
	fn validate_utxo_output(&mut self, out: &Output) -> Result<(), Error> {
		let commit = out.commitment();
		if let Ok(pos) = self.store.get_output_pos(&commit) {
			if let Some(out_mmr) = self.output_pmmr.get_data(pos) {
				if out_mmr.commitment() == commit {
					return Err(ErrorKind::DuplicateCommitment(commit).into());
				}
			}
		}
		Ok(())
	}

	/// Rewinds the MMRs to the provided block, rewinding to the last output pos
	/// and last kernel pos of that block.
	pub fn rewind(&mut self, block_header: &BlockHeader) -> Result<(), Error> {
		trace!(
			LOGGER,
			"Rewind to header {} @ {}",
			block_header.height,
			block_header.hash(),
		);

		let head_header = self.store.head_header()?;

		// We need to build bitmaps of added and removed output positions
		// so we can correctly rewind all operations applied to the output MMR
		// after the position we are rewinding to (these operations will be
		// undone during rewind).
		// Rewound output pos will be removed from the MMR.
		// Rewound input (spent) pos will be added back to the MMR.
		let rewind_rm_pos = input_pos_to_rewind(
			self.store.clone(),
			block_header,
			&head_header,
			&self.batch,
		)?;

		self.rewind_to_pos(
			block_header.output_mmr_size,
			&rewind_rm_pos,
		)
	}

	/// Rewinds the MMRs to the provided positions, given the output and
	/// kernel we want to rewind to.
	fn rewind_to_pos(
		&mut self,
		output_pos: u64,
		rewind_rm_pos: &Bitmap,
	) -> Result<(), Error> {
		trace!(
			LOGGER,
			"Rewind utxo_view to output {}",
			output_pos,
		);

		self.output_pmmr
			.rewind(output_pos, rewind_rm_pos)
			.map_err(&ErrorKind::TxHashSetErr)?;
		Ok(())
	}
}
