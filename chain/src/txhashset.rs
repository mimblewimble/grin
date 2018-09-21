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

//! Utility structs to handle the 3 hashtrees (output, range proof,
//! kernel) more conveniently and transactionally.

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
use core::core::{
	Block, BlockHeader, Input, Output, OutputFeatures, OutputIdentifier, Transaction, TxKernel,
};
use core::global;
use core::ser::{PMMRIndexHashable, PMMRable};

use error::{Error, ErrorKind};
use grin_store;
use grin_store::pmmr::{PMMRBackend, PMMR_FILES};
use grin_store::types::prune_noop;
use store::{Batch, ChainStore};
use types::{TxHashSetRoots, TxHashsetWriteStatus};
use util::{file, secp_static, zip, LOGGER};

const TXHASHSET_SUBDIR: &'static str = "txhashset";
const OUTPUT_SUBDIR: &'static str = "output";
const RANGE_PROOF_SUBDIR: &'static str = "rangeproof";
const KERNEL_SUBDIR: &'static str = "kernel";
const TXHASHSET_ZIP: &'static str = "txhashset_snapshot.zip";

struct PMMRHandle<T>
where
	T: PMMRable,
{
	backend: PMMRBackend<T>,
	last_pos: u64,
}

impl<T> PMMRHandle<T>
where
	T: PMMRable + ::std::fmt::Debug,
{
	fn new(
		root_dir: String,
		file_name: &str,
		prunable: bool,
		header: Option<&BlockHeader>,
	) -> Result<PMMRHandle<T>, Error> {
		let path = Path::new(&root_dir).join(TXHASHSET_SUBDIR).join(file_name);
		fs::create_dir_all(path.clone())?;
		let be = PMMRBackend::new(path.to_str().unwrap().to_string(), prunable, header)?;
		let sz = be.unpruned_size()?;
		Ok(PMMRHandle {
			backend: be,
			last_pos: sz,
		})
	}
}

/// An easy to manipulate structure holding the 3 sum trees necessary to
/// validate blocks and capturing the Output set, the range proofs and the
/// kernels. Also handles the index of Commitments to positions in the
/// output and range proof pmmr trees.
///
/// Note that the index is never authoritative, only the trees are
/// guaranteed to indicate whether an output is spent or not. The index
/// may have commitments that have already been spent, even with
/// pruning enabled.

pub struct TxHashSet {
	output_pmmr_h: PMMRHandle<OutputIdentifier>,
	rproof_pmmr_h: PMMRHandle<RangeProof>,
	kernel_pmmr_h: PMMRHandle<TxKernel>,

	// chain store used as index of commitments to MMR positions
	commit_index: Arc<ChainStore>,
}

impl TxHashSet {
	/// Open an existing or new set of backends for the TxHashSet
	pub fn open(
		root_dir: String,
		commit_index: Arc<ChainStore>,
		header: Option<&BlockHeader>,
	) -> Result<TxHashSet, Error> {
		let output_file_path: PathBuf = [&root_dir, TXHASHSET_SUBDIR, OUTPUT_SUBDIR]
			.iter()
			.collect();
		fs::create_dir_all(output_file_path.clone())?;

		let rproof_file_path: PathBuf = [&root_dir, TXHASHSET_SUBDIR, RANGE_PROOF_SUBDIR]
			.iter()
			.collect();
		fs::create_dir_all(rproof_file_path.clone())?;

		let kernel_file_path: PathBuf = [&root_dir, TXHASHSET_SUBDIR, KERNEL_SUBDIR]
			.iter()
			.collect();
		fs::create_dir_all(kernel_file_path.clone())?;

		Ok(TxHashSet {
			output_pmmr_h: PMMRHandle::new(root_dir.clone(), OUTPUT_SUBDIR, true, header)?,
			rproof_pmmr_h: PMMRHandle::new(root_dir.clone(), RANGE_PROOF_SUBDIR, true, header)?,
			kernel_pmmr_h: PMMRHandle::new(root_dir.clone(), KERNEL_SUBDIR, false, None)?,
			commit_index,
		})
	}

	/// Check if an output is unspent.
	/// We look in the index to find the output MMR pos.
	/// Then we check the entry in the output MMR and confirm the hash matches.
	pub fn is_unspent(&mut self, output_id: &OutputIdentifier) -> Result<(Hash, u64), Error> {
		match self.commit_index.get_output_pos(&output_id.commit) {
			Ok(pos) => {
				let output_pmmr: PMMR<OutputIdentifier, _> =
					PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
				if let Some(hash) = output_pmmr.get_hash(pos) {
					if hash == output_id.hash_with_index(pos - 1) {
						Ok((hash, pos))
					} else {
						Err(ErrorKind::TxHashSetErr(format!("txhashset hash mismatch")).into())
					}
				} else {
					Err(ErrorKind::OutputNotFound.into())
				}
			}
			Err(grin_store::Error::NotFoundErr(_)) => Err(ErrorKind::OutputNotFound.into()),
			Err(e) => Err(ErrorKind::StoreErr(e, format!("txhashset unspent check")).into()),
		}
	}

	/// returns the last N nodes inserted into the tree (i.e. the 'bottom'
	/// nodes at level 0
	/// TODO: These need to return the actual data from the flat-files instead
	/// of hashes now
	pub fn last_n_output(&mut self, distance: u64) -> Vec<(Hash, OutputIdentifier)> {
		let output_pmmr: PMMR<OutputIdentifier, _> =
			PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
		output_pmmr.get_last_n_insertions(distance)
	}

	/// as above, for range proofs
	pub fn last_n_rangeproof(&mut self, distance: u64) -> Vec<(Hash, RangeProof)> {
		let rproof_pmmr: PMMR<RangeProof, _> =
			PMMR::at(&mut self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos);
		rproof_pmmr.get_last_n_insertions(distance)
	}

	/// as above, for kernels
	pub fn last_n_kernel(&mut self, distance: u64) -> Vec<(Hash, TxKernel)> {
		let kernel_pmmr: PMMR<TxKernel, _> =
			PMMR::at(&mut self.kernel_pmmr_h.backend, self.kernel_pmmr_h.last_pos);
		kernel_pmmr.get_last_n_insertions(distance)
	}

	/// returns outputs from the given insertion (leaf) index up to the
	/// specified limit. Also returns the last index actually populated
	pub fn outputs_by_insertion_index(
		&mut self,
		start_index: u64,
		max_count: u64,
	) -> (u64, Vec<OutputIdentifier>) {
		let output_pmmr: PMMR<OutputIdentifier, _> =
			PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
		output_pmmr.elements_from_insertion_index(start_index, max_count)
	}

	/// highest output insertion index available
	pub fn highest_output_insertion_index(&mut self) -> u64 {
		pmmr::n_leaves(self.output_pmmr_h.last_pos)
	}

	/// As above, for rangeproofs
	pub fn rangeproofs_by_insertion_index(
		&mut self,
		start_index: u64,
		max_count: u64,
	) -> (u64, Vec<RangeProof>) {
		let rproof_pmmr: PMMR<RangeProof, _> =
			PMMR::at(&mut self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos);
		rproof_pmmr.elements_from_insertion_index(start_index, max_count)
	}

	/// Get sum tree roots
	/// TODO: Return data instead of hashes
	pub fn roots(&mut self) -> (Hash, Hash, Hash) {
		let output_pmmr: PMMR<OutputIdentifier, _> =
			PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
		let rproof_pmmr: PMMR<RangeProof, _> =
			PMMR::at(&mut self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos);
		let kernel_pmmr: PMMR<TxKernel, _> =
			PMMR::at(&mut self.kernel_pmmr_h.backend, self.kernel_pmmr_h.last_pos);
		(output_pmmr.root(), rproof_pmmr.root(), kernel_pmmr.root())
	}

	/// build a new merkle proof for the given position
	pub fn merkle_proof(&mut self, commit: Commitment) -> Result<MerkleProof, String> {
		let pos = self.commit_index.get_output_pos(&commit).unwrap();
		let output_pmmr: PMMR<OutputIdentifier, _> =
			PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
		output_pmmr.merkle_proof(pos)
	}

	/// Compact the MMR data files and flush the rm logs
	pub fn compact(&mut self) -> Result<(), Error> {
		let commit_index = self.commit_index.clone();
		let head_header = commit_index.head_header()?;
		let current_height = head_header.height;

		// horizon for compacting is based on current_height
		let horizon = current_height.saturating_sub(global::cut_through_horizon().into());
		let horizon_header = self.commit_index.get_header_by_height(horizon)?;

		let batch = self.commit_index.batch()?;

		let rewind_rm_pos = input_pos_to_rewind(
			self.commit_index.clone(),
			&horizon_header,
			&head_header,
			&batch,
		)?;

		{
			let clean_output_index = |commit: &[u8]| {
				let _ = batch.delete_output_pos(commit);
			};

			self.output_pmmr_h.backend.check_compact(
				horizon_header.output_mmr_size,
				&rewind_rm_pos,
				clean_output_index,
			)?;

			self.rproof_pmmr_h.backend.check_compact(
				horizon_header.output_mmr_size,
				&rewind_rm_pos,
				&prune_noop,
			)?;
		}

		// Finally commit the batch, saving everything to the db.
		batch.commit()?;

		Ok(())
	}
}

/// Starts a new unit of work to extend (or rewind) the chain with additional
/// blocks. Accepts a closure that will operate within that unit of work.
/// The closure has access to an Extension object that allows the addition
/// of blocks to the txhashset and the checking of the current tree roots.
///
/// The unit of work is always discarded (always rollback) as this is read-only.
pub fn extending_readonly<'a, F, T>(trees: &'a mut TxHashSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&mut Extension) -> Result<T, Error>,
{
	let res: Result<T, Error>;
	{
		let commit_index = trees.commit_index.clone();
		let commit_index2 = trees.commit_index.clone();
		let batch = commit_index.batch()?;

		trace!(LOGGER, "Starting new txhashset (readonly) extension.");
		let mut extension = Extension::new(trees, &batch, commit_index2);
		extension.force_rollback();
		res = inner(&mut extension);
	}

	trace!(LOGGER, "Rollbacking txhashset (readonly) extension.");

	trees.output_pmmr_h.backend.discard();
	trees.rproof_pmmr_h.backend.discard();
	trees.kernel_pmmr_h.backend.discard();

	trace!(LOGGER, "TxHashSet (readonly) extension done.");

	res
}

/// Starts a new unit of work to extend the chain with additional blocks,
/// accepting a closure that will work within that unit of work. The closure
/// has access to an Extension object that allows the addition of blocks to
/// the txhashset and the checking of the current tree roots.
///
/// If the closure returns an error, modifications are canceled and the unit
/// of work is abandoned. Otherwise, the unit of work is permanently applied.
pub fn extending<'a, F, T>(
	trees: &'a mut TxHashSet,
	batch: &'a mut Batch,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut Extension) -> Result<T, Error>,
{
	let sizes: (u64, u64, u64);
	let res: Result<T, Error>;
	let rollback: bool;

	// create a child transaction so if the state is rolled back by itself, all
	// index saving can be undone
	let child_batch = batch.child()?;
	{
		let commit_index = trees.commit_index.clone();

		trace!(LOGGER, "Starting new txhashset extension.");
		let mut extension = Extension::new(trees, &child_batch, commit_index);
		res = inner(&mut extension);

		rollback = extension.rollback;
		sizes = extension.sizes();
	}

	match res {
		Err(e) => {
			debug!(
				LOGGER,
				"Error returned, discarding txhashset extension: {}", e
			);
			trees.output_pmmr_h.backend.discard();
			trees.rproof_pmmr_h.backend.discard();
			trees.kernel_pmmr_h.backend.discard();
			Err(e)
		}
		Ok(r) => {
			if rollback {
				trace!(LOGGER, "Rollbacking txhashset extension. sizes {:?}", sizes);
				trees.output_pmmr_h.backend.discard();
				trees.rproof_pmmr_h.backend.discard();
				trees.kernel_pmmr_h.backend.discard();
			} else {
				trace!(LOGGER, "Committing txhashset extension. sizes {:?}", sizes);
				child_batch.commit()?;
				trees.output_pmmr_h.backend.sync()?;
				trees.rproof_pmmr_h.backend.sync()?;
				trees.kernel_pmmr_h.backend.sync()?;
				trees.output_pmmr_h.last_pos = sizes.0;
				trees.rproof_pmmr_h.last_pos = sizes.1;
				trees.kernel_pmmr_h.last_pos = sizes.2;
			}

			trace!(LOGGER, "TxHashSet extension done.");
			Ok(r)
		}
	}
}

/// Allows the application of new blocks on top of the sum trees in a
/// reversible manner within a unit of work provided by the `extending`
/// function.
pub struct Extension<'a> {
	output_pmmr: PMMR<'a, OutputIdentifier, PMMRBackend<OutputIdentifier>>,
	rproof_pmmr: PMMR<'a, RangeProof, PMMRBackend<RangeProof>>,
	kernel_pmmr: PMMR<'a, TxKernel, PMMRBackend<TxKernel>>,

	commit_index: Arc<ChainStore>,
	rollback: bool,

	/// Batch in which the extension occurs, public so it can be used within
	/// and `extending` closure. Just be careful using it that way as it will
	/// get rolled back with the extension (i.e on a losing fork).
	pub batch: &'a Batch<'a>,
}

impl<'a> Committed for Extension<'a> {
	fn inputs_committed(&self) -> Vec<Commitment> {
		vec![]
	}

	fn outputs_committed(&self) -> Vec<Commitment> {
		let mut commitments = vec![];
		for n in 1..self.output_pmmr.unpruned_size() + 1 {
			if pmmr::is_leaf(n) {
				if let Some(out) = self.output_pmmr.get_data(n) {
					commitments.push(out.commit);
				}
			}
		}
		commitments
	}

	fn kernels_committed(&self) -> Vec<Commitment> {
		let mut commitments = vec![];
		for n in 1..self.kernel_pmmr.unpruned_size() + 1 {
			if pmmr::is_leaf(n) {
				if let Some(kernel) = self.kernel_pmmr.get_data(n) {
					commitments.push(kernel.excess);
				}
			}
		}
		commitments
	}
}

impl<'a> Extension<'a> {
	// constructor
	fn new(
		trees: &'a mut TxHashSet,
		batch: &'a Batch,
		commit_index: Arc<ChainStore>,
	) -> Extension<'a> {
		Extension {
			output_pmmr: PMMR::at(
				&mut trees.output_pmmr_h.backend,
				trees.output_pmmr_h.last_pos,
			),
			rproof_pmmr: PMMR::at(
				&mut trees.rproof_pmmr_h.backend,
				trees.rproof_pmmr_h.last_pos,
			),
			kernel_pmmr: PMMR::at(
				&mut trees.kernel_pmmr_h.backend,
				trees.kernel_pmmr_h.last_pos,
			),
			commit_index,
			rollback: false,
			batch,
		}
	}

	// Rewind the MMR backend to undo applying a raw tx to the txhashset extension.
	// This is used during txpool validation to undo an invalid tx.
	fn rewind_raw_tx(
		&mut self,
		output_pos: u64,
		kernel_pos: u64,
		rewind_rm_pos: &Bitmap,
	) -> Result<(), Error> {
		self.rewind_to_pos(output_pos, kernel_pos, rewind_rm_pos)?;
		Ok(())
	}

	/// Apply a "raw" transaction to the txhashset.
	/// We will never commit a txhashset extension that includes raw txs.
	/// But we can use this when validating txs in the tx pool.
	/// If we can add a tx to the tx pool and then successfully add the
	/// aggregated tx from the tx pool to the current chain state (via a
	/// txhashset extension) then we know the tx pool is valid (including the
	/// new tx).
	pub fn apply_raw_tx(&mut self, tx: &Transaction) -> Result<(), Error> {
		// This should *never* be called on a writeable extension...
		assert!(
			self.rollback,
			"applied raw_tx to writeable txhashset extension"
		);

		// Checkpoint the MMR positions before we apply the new tx,
		// anything goes wrong we will rewind to these positions.
		let output_pos = self.output_pmmr.unpruned_size();
		let kernel_pos = self.kernel_pmmr.unpruned_size();

		// Build bitmap of output pos spent (as inputs) by this tx for rewind.
		let rewind_rm_pos = tx
			.inputs()
			.iter()
			.filter_map(|x| self.batch.get_output_pos(&x.commitment()).ok())
			.map(|x| x as u32)
			.collect();

		for ref output in tx.outputs() {
			match self.apply_output(output) {
				Ok(pos) => {
					self.batch.save_output_pos(&output.commitment(), pos)?;
				}
				Err(e) => {
					self.rewind_raw_tx(output_pos, kernel_pos, &rewind_rm_pos)?;
					return Err(e);
				}
			}
		}

		for ref input in tx.inputs() {
			if let Err(e) = self.apply_input(input) {
				self.rewind_raw_tx(output_pos, kernel_pos, &rewind_rm_pos)?;
				return Err(e);
			}
		}

		for ref kernel in tx.kernels() {
			if let Err(e) = self.apply_kernel(kernel) {
				self.rewind_raw_tx(output_pos, kernel_pos, &rewind_rm_pos)?;
				return Err(e);
			}
		}

		Ok(())
	}

	/// Validate a vector of "raw" transactions against the current chain state.
	/// We support rewind on a "dirty" txhashset - so we can apply each tx in
	/// turn, rewinding if any particular tx is not valid and continuing
	/// through the vec of txs provided. This allows us to efficiently apply
	/// all the txs, filtering out those that are not valid and returning the
	/// final vec of txs that were successfully validated against the txhashset.
	///
	/// Note: We also pass in a "pre_tx". This tx is applied to and validated
	/// before we start applying the vec of txs. We use this when validating
	/// txs in the stempool as we need to account for txs in the txpool as
	/// well (new_tx + stempool + txpool + txhashset). So we aggregate the
	/// contents of the txpool into a single aggregated tx and pass it in here
	/// as the "pre_tx" so we apply it to the txhashset before we start
	/// validating the stempool txs.
	/// This is optional and we pass in None when validating the txpool txs
	/// themselves.
	///
	pub fn validate_raw_txs(
		&mut self,
		txs: Vec<Transaction>,
		pre_tx: Option<Transaction>,
	) -> Result<Vec<Transaction>, Error> {
		let mut valid_txs = vec![];

		// First apply the "pre_tx" to account for any state that need adding to
		// the chain state before we can validate our vec of txs.
		// This is the aggregate tx from the txpool if we are validating the stempool.
		if let Some(tx) = pre_tx {
			self.apply_raw_tx(&tx)?;
		}

		// Now validate each tx, rewinding any tx (and only that tx)
		// if it fails to validate successfully.
		for tx in txs {
			if self.apply_raw_tx(&tx).is_ok() {
				valid_txs.push(tx);
			}
		}
		Ok(valid_txs)
	}

	/// Verify we are not attempting to spend any coinbase outputs
	/// that have not sufficiently matured.
	pub fn verify_coinbase_maturity(
		&mut self,
		inputs: &Vec<Input>,
		height: u64,
	) -> Result<(), Error> {
		// Find the greatest output pos of any coinbase
		// outputs we are attempting to spend.
		let pos = inputs
			.iter()
			.filter(|x| x.features.contains(OutputFeatures::COINBASE_OUTPUT))
			.filter_map(|x| self.commit_index.get_output_pos(&x.commitment()).ok())
			.max()
			.unwrap_or(0);

		if pos > 0 {
			// If we have not yet reached 1,000 / 1,440 blocks then
			// we can fail immediately as coinbase cannot be mature.
			if height < global::coinbase_maturity(height) {
				return Err(ErrorKind::ImmatureCoinbase.into());
			}

			// Find the "cutoff" pos in the output MMR based on the
			// header from 1,000 blocks ago.
			let cutoff_height = height
				.checked_sub(global::coinbase_maturity(height))
				.unwrap_or(0);
			let cutoff_header = self.commit_index.get_header_by_height(cutoff_height)?;
			let cutoff_pos = cutoff_header.output_mmr_size;

			// If any output pos exceed the cutoff_pos
			// we know they have not yet sufficiently matured.
			if pos > cutoff_pos {
				return Err(ErrorKind::ImmatureCoinbase.into());
			}
		}

		Ok(())
	}

	/// Apply a new set of blocks on top the existing sum trees. Blocks are
	/// applied in order of the provided Vec. If pruning is enabled, inputs also
	/// prune MMR data.
	pub fn apply_block(&mut self, b: &Block) -> Result<(), Error> {
		for out in b.outputs() {
			let pos = self.apply_output(out)?;
			// Update the output_pos index for the new output.
			self.batch.save_output_pos(&out.commitment(), pos)?;
		}

		for input in b.inputs() {
			self.apply_input(input)?;
		}

		for kernel in b.kernels() {
			self.apply_kernel(kernel)?;
		}

		Ok(())
	}

	fn apply_input(&mut self, input: &Input) -> Result<(), Error> {
		let commit = input.commitment();
		let pos_res = self.batch.get_output_pos(&commit);
		if let Ok(pos) = pos_res {
			let output_id_hash = OutputIdentifier::from_input(input).hash_with_index(pos - 1);
			if let Some(read_hash) = self.output_pmmr.get_hash(pos) {
				// check hash from pmmr matches hash from input (or corresponding output)
				// if not then the input is not being honest about
				// what it is attempting to spend...
				let read_elem = self.output_pmmr.get_data(pos);
				let read_elem_hash = read_elem
					.expect("no output at pos")
					.hash_with_index(pos - 1);
				if output_id_hash != read_hash || output_id_hash != read_elem_hash {
					return Err(
						ErrorKind::TxHashSetErr(format!("output pmmr hash mismatch")).into(),
					);
				}
			}

			// Now prune the output_pmmr, rproof_pmmr and their storage.
			// Input is not valid if we cannot prune successfully (to spend an unspent
			// output).
			match self.output_pmmr.prune(pos) {
				Ok(true) => {
					self.rproof_pmmr
						.prune(pos)
						.map_err(|s| ErrorKind::TxHashSetErr(s))?;
				}
				Ok(false) => return Err(ErrorKind::AlreadySpent(commit).into()),
				Err(s) => return Err(ErrorKind::TxHashSetErr(s).into()),
			}
		} else {
			return Err(ErrorKind::AlreadySpent(commit).into());
		}
		Ok(())
	}

	fn apply_output(&mut self, out: &Output) -> Result<(u64), Error> {
		let commit = out.commitment();

		if let Ok(pos) = self.batch.get_output_pos(&commit) {
			if let Some(out_mmr) = self.output_pmmr.get_data(pos) {
				if out_mmr.commitment() == commit {
					return Err(ErrorKind::DuplicateCommitment(commit).into());
				}
			}
		}
		// push the new output to the MMR.
		let output_pos = self
			.output_pmmr
			.push(OutputIdentifier::from_output(out))
			.map_err(&ErrorKind::TxHashSetErr)?;

		// push the rangeproof to the MMR.
		let rproof_pos = self
			.rproof_pmmr
			.push(out.proof)
			.map_err(&ErrorKind::TxHashSetErr)?;

		// The output and rproof MMRs should be exactly the same size
		// and we should have inserted to both in exactly the same pos.
		{
			if self.output_pmmr.unpruned_size() != self.rproof_pmmr.unpruned_size() {
				return Err(
					ErrorKind::Other(format!("output vs rproof MMRs different sizes")).into(),
				);
			}

			if output_pos != rproof_pos {
				return Err(ErrorKind::Other(format!("output vs rproof MMRs different pos")).into());
			}
		}

		Ok(output_pos)
	}

	fn apply_kernel(&mut self, kernel: &TxKernel) -> Result<(), Error> {
		// push kernels in their MMR and file
		self.kernel_pmmr
			.push(kernel.clone())
			.map_err(&ErrorKind::TxHashSetErr)?;

		Ok(())
	}

	/// Build a Merkle proof for the given output and the block by
	/// rewinding the MMR to the last pos of the block.
	/// Note: this relies on the MMR being stable even after pruning/compaction.
	/// We need the hash of each sibling pos from the pos up to the peak
	/// including the sibling leaf node which may have been removed.
	pub fn merkle_proof(
		&mut self,
		output: &OutputIdentifier,
		block_header: &BlockHeader,
	) -> Result<MerkleProof, Error> {
		debug!(
			LOGGER,
			"txhashset: merkle_proof: output: {:?}, block: {:?}",
			output.commit,
			block_header.hash()
		);

		// rewind to the specified block for a consistent view
		self.rewind(block_header)?;

		// then calculate the Merkle Proof based on the known pos
		let pos = self.batch.get_output_pos(&output.commit)?;
		let merkle_proof = self
			.output_pmmr
			.merkle_proof(pos)
			.map_err(&ErrorKind::TxHashSetErr)?;

		Ok(merkle_proof)
	}

	/// Saves a snapshot of the output and rangeproof MMRs to disk.
	/// Specifically - saves a snapshot of the utxo file, tagged with
	/// the block hash as filename suffix.
	/// Needed for fast-sync (utxo file needs to be rewound before sending
	/// across).
	pub fn snapshot(&mut self, header: &BlockHeader) -> Result<(), Error> {
		self.output_pmmr
			.snapshot(header)
			.map_err(|e| ErrorKind::Other(e))?;
		self.rproof_pmmr
			.snapshot(header)
			.map_err(|e| ErrorKind::Other(e))?;
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

		let head_header = self.commit_index.head_header()?;

		// We need to build bitmaps of added and removed output positions
		// so we can correctly rewind all operations applied to the output MMR
		// after the position we are rewinding to (these operations will be
		// undone during rewind).
		// Rewound output pos will be removed from the MMR.
		// Rewound input (spent) pos will be added back to the MMR.
		let rewind_rm_pos = input_pos_to_rewind(
			self.commit_index.clone(),
			block_header,
			&head_header,
			&self.batch,
		)?;

		self.rewind_to_pos(
			block_header.output_mmr_size,
			block_header.kernel_mmr_size,
			&rewind_rm_pos,
		)
	}

	/// Rewinds the MMRs to the provided positions, given the output and
	/// kernel we want to rewind to.
	fn rewind_to_pos(
		&mut self,
		output_pos: u64,
		kernel_pos: u64,
		rewind_rm_pos: &Bitmap,
	) -> Result<(), Error> {
		trace!(
			LOGGER,
			"Rewind txhashset to output {}, kernel {}",
			output_pos,
			kernel_pos,
		);

		self.output_pmmr
			.rewind(output_pos, rewind_rm_pos)
			.map_err(&ErrorKind::TxHashSetErr)?;
		self.rproof_pmmr
			.rewind(output_pos, rewind_rm_pos)
			.map_err(&ErrorKind::TxHashSetErr)?;
		self.kernel_pmmr
			.rewind(kernel_pos, &Bitmap::create())
			.map_err(&ErrorKind::TxHashSetErr)?;
		Ok(())
	}

	/// Current root hashes and sums (if applicable) for the Output, range proof
	/// and kernel sum trees.
	pub fn roots(&self) -> TxHashSetRoots {
		TxHashSetRoots {
			output_root: self.output_pmmr.root(),
			rproof_root: self.rproof_pmmr.root(),
			kernel_root: self.kernel_pmmr.root(),
		}
	}

	/// Validate the various MMR roots against the block header.
	pub fn validate_roots(&self, header: &BlockHeader) -> Result<(), Error> {
		// If we are validating the genesis block then we have no outputs or
		// kernels. So we are done here.
		if header.height == 0 {
			return Ok(());
		}

		let roots = self.roots();
		if roots.output_root != header.output_root
			|| roots.rproof_root != header.range_proof_root
			|| roots.kernel_root != header.kernel_root
		{
			Err(ErrorKind::InvalidRoot.into())
		} else {
			Ok(())
		}
	}

	/// Validate the output and kernel MMR sizes against the block header.
	pub fn validate_sizes(&self, header: &BlockHeader) -> Result<(), Error> {
		// If we are validating the genesis block then we have no outputs or
		// kernels. So we are done here.
		if header.height == 0 {
			return Ok(());
		}

		let (output_mmr_size, _, kernel_mmr_size) = self.sizes();
		if output_mmr_size != header.output_mmr_size || kernel_mmr_size != header.kernel_mmr_size {
			Err(ErrorKind::InvalidMMRSize.into())
		} else {
			Ok(())
		}
	}

	fn validate_mmrs(&self) -> Result<(), Error> {
		let now = Instant::now();

		// validate all hashes and sums within the trees
		if let Err(e) = self.output_pmmr.validate() {
			return Err(ErrorKind::InvalidTxHashSet(e).into());
		}
		if let Err(e) = self.rproof_pmmr.validate() {
			return Err(ErrorKind::InvalidTxHashSet(e).into());
		}
		if let Err(e) = self.kernel_pmmr.validate() {
			return Err(ErrorKind::InvalidTxHashSet(e).into());
		}

		debug!(
			LOGGER,
			"txhashset: validated the output {}, rproof {}, kernel {} mmrs, took {}s",
			self.output_pmmr.unpruned_size(),
			self.rproof_pmmr.unpruned_size(),
			self.kernel_pmmr.unpruned_size(),
			now.elapsed().as_secs(),
		);

		Ok(())
	}

	/// Validate full kernel sums against the provided header (for overage and kernel_offset).
	/// This is an expensive operation as we need to retrieve all the UTXOs and kernels
	/// from the respective MMRs.
	/// For a significantly faster way of validating full kernel sums see BlockSums.
	pub fn validate_kernel_sums(
		&self,
		header: &BlockHeader,
	) -> Result<((Commitment, Commitment)), Error> {
		let (utxo_sum, kernel_sum) =
			self.verify_kernel_sums(header.total_overage(), header.total_kernel_offset())?;
		Ok((utxo_sum, kernel_sum))
	}

	/// Validate the txhashset state against the provided block header.
	pub fn validate(
		&mut self,
		header: &BlockHeader,
		skip_rproofs: bool,
		status: &TxHashsetWriteStatus,
	) -> Result<((Commitment, Commitment)), Error> {
		self.validate_mmrs()?;
		self.validate_roots(header)?;
		self.validate_sizes(header)?;

		if header.height == 0 {
			let zero_commit = secp_static::commit_to_zero_value();
			return Ok((zero_commit.clone(), zero_commit.clone()));
		}

		// The real magicking happens here. Sum of kernel excesses should equal
		// sum of unspent outputs minus total supply.
		let (output_sum, kernel_sum) = self.validate_kernel_sums(header)?;

		// This is an expensive verification step.
		self.verify_kernel_signatures(status)?;

		// Verify the rangeproof for each output in the sum above.
		// This is an expensive verification step (skip for faster verification).
		if !skip_rproofs {
			self.verify_rangeproofs(status)?;
		}

		Ok((output_sum, kernel_sum))
	}

	/// Rebuild the index of MMR positions to the corresponding Output and
	/// kernel by iterating over the whole MMR data. This is a costly operation
	/// performed only when we receive a full new chain state.
	pub fn rebuild_index(&self) -> Result<(), Error> {
		for n in 1..self.output_pmmr.unpruned_size() + 1 {
			// non-pruned leaves only
			if pmmr::bintree_postorder_height(n) == 0 {
				if let Some(out) = self.output_pmmr.get_data(n) {
					self.batch.save_output_pos(&out.commit, n)?;
				}
			}
		}
		Ok(())
	}

	/// Force the rollback of this extension, no matter the result
	pub fn force_rollback(&mut self) {
		self.rollback = true;
	}

	/// Dumps the output MMR.
	/// We use this after compacting for visual confirmation that it worked.
	pub fn dump_output_pmmr(&self) {
		debug!(LOGGER, "-- outputs --");
		self.output_pmmr.dump_from_file(false);
		debug!(LOGGER, "--");
		self.output_pmmr.dump_stats();
		debug!(LOGGER, "-- end of outputs --");
	}

	/// Dumps the state of the 3 sum trees to stdout for debugging. Short
	/// version only prints the Output tree.
	pub fn dump(&self, short: bool) {
		debug!(LOGGER, "-- outputs --");
		self.output_pmmr.dump(short);
		if !short {
			debug!(LOGGER, "-- range proofs --");
			self.rproof_pmmr.dump(short);
			debug!(LOGGER, "-- kernels --");
			self.kernel_pmmr.dump(short);
		}
	}

	/// Sizes of each of the sum trees
	pub fn sizes(&self) -> (u64, u64, u64) {
		(
			self.output_pmmr.unpruned_size(),
			self.rproof_pmmr.unpruned_size(),
			self.kernel_pmmr.unpruned_size(),
		)
	}

	fn verify_kernel_signatures(&self, status: &TxHashsetWriteStatus) -> Result<(), Error> {
		let now = Instant::now();

		let mut kern_count = 0;
		let total_kernels = pmmr::n_leaves(self.kernel_pmmr.unpruned_size());
		for n in 1..self.kernel_pmmr.unpruned_size() + 1 {
			if pmmr::is_leaf(n) {
				if let Some(kernel) = self.kernel_pmmr.get_data(n) {
					kernel.verify()?;
					kern_count += 1;
				}
			}
			if n % 20 == 0 {
				status.on_validation(kern_count, total_kernels, 0, 0);
			}
		}

		debug!(
			LOGGER,
			"txhashset: verified {} kernel signatures, pmmr size {}, took {}s",
			kern_count,
			self.kernel_pmmr.unpruned_size(),
			now.elapsed().as_secs(),
		);

		Ok(())
	}

	fn verify_rangeproofs(&self, status: &TxHashsetWriteStatus) -> Result<(), Error> {
		let now = Instant::now();

		let mut commits: Vec<Commitment> = vec![];
		let mut proofs: Vec<RangeProof> = vec![];

		let mut proof_count = 0;
		let total_rproofs = pmmr::n_leaves(self.output_pmmr.unpruned_size());
		for n in 1..self.output_pmmr.unpruned_size() + 1 {
			if pmmr::is_leaf(n) {
				if let Some(out) = self.output_pmmr.get_data(n) {
					if let Some(rp) = self.rproof_pmmr.get_data(n) {
						commits.push(out.commit);
						proofs.push(rp);
					} else {
						// TODO - rangeproof not found
						return Err(ErrorKind::OutputNotFound.into());
					}
					proof_count += 1;

					if proofs.len() >= 1000 {
						Output::batch_verify_proofs(&commits, &proofs)?;
						commits.clear();
						proofs.clear();
						debug!(
							LOGGER,
							"txhashset: verify_rangeproofs: verified {} rangeproofs", proof_count,
						);
					}
				}
			}
			if n % 20 == 0 {
				status.on_validation(0, 0, proof_count, total_rproofs);
			}
		}

		// remaining part which not full of 1000 range proofs
		if proofs.len() > 0 {
			Output::batch_verify_proofs(&commits, &proofs)?;
			commits.clear();
			proofs.clear();
			debug!(
				LOGGER,
				"txhashset: verify_rangeproofs: verified {} rangeproofs", proof_count,
			);
		}

		debug!(
			LOGGER,
			"txhashset: verified {} rangeproofs, pmmr size {}, took {}s",
			proof_count,
			self.rproof_pmmr.unpruned_size(),
			now.elapsed().as_secs(),
		);
		Ok(())
	}

	/// Special handling to make sure the whole kernel set matches each of its
	/// roots in each block header, without truncation. We go back header by
	/// header, rewind and check each root. This fixes a potential weakness in
	/// fast sync where a reorg past the horizon could allow a whole rewrite of
	/// the kernel set.
	pub fn validate_kernel_history(&mut self, header: &BlockHeader) -> Result<(), Error> {
		assert!(
			self.rollback,
			"verified kernel history on writeable txhashset extension"
		);

		let mut current = header.clone();
		loop {
			current = self.commit_index.get_block_header(&current.previous)?;
			if current.height == 0 {
				break;
			}
			// rewinding kernels only further and further back
			self.kernel_pmmr
				.rewind(current.kernel_mmr_size, &Bitmap::create())
				.map_err(&ErrorKind::TxHashSetErr)?;
			if self.kernel_pmmr.root() != current.kernel_root {
				return Err(ErrorKind::InvalidTxHashSet(format!(
					"Kernel root at {} does not match",
					current.height
				)).into());
			}
		}
		Ok(())
	}
}

/// Packages the txhashset data files into a zip and returns a Read to the
/// resulting file
pub fn zip_read(root_dir: String, header: &BlockHeader) -> Result<File, Error> {
	let txhashset_path = Path::new(&root_dir).join(TXHASHSET_SUBDIR);
	let zip_path = Path::new(&root_dir).join(TXHASHSET_ZIP);
	// create the zip archive
	{
		// Temp txhashset directory
		let temp_txhashset_path = Path::new(&root_dir).join(TXHASHSET_SUBDIR.to_string() + "_zip");
		// Remove temp dir if it exist
		if temp_txhashset_path.exists() {
			fs::remove_dir_all(&temp_txhashset_path)?;
		}
		// Copy file to another dir
		file::copy_dir_to(&txhashset_path, &temp_txhashset_path)?;
		// Check and remove file that are not supposed to be there
		check_and_remove_files(&temp_txhashset_path, header)?;
		// Compress zip
		zip::compress(&temp_txhashset_path, &File::create(zip_path.clone())?)
			.map_err(|ze| ErrorKind::Other(ze.to_string()))?;
	}

	// open it again to read it back
	let zip_file = File::open(zip_path)?;
	Ok(zip_file)
}

/// Extract the txhashset data from a zip file and writes the content into the
/// txhashset storage dir
pub fn zip_write(
	root_dir: String,
	txhashset_data: File,
	header: &BlockHeader,
) -> Result<(), Error> {
	let txhashset_path = Path::new(&root_dir).join(TXHASHSET_SUBDIR);
	fs::create_dir_all(txhashset_path.clone())?;
	zip::decompress(txhashset_data, &txhashset_path)
		.map_err(|ze| ErrorKind::Other(ze.to_string()))?;
	check_and_remove_files(&txhashset_path, header)
}

/// Check a txhashset directory and remove any unexpected
fn check_and_remove_files(txhashset_path: &PathBuf, header: &BlockHeader) -> Result<(), Error> {
	// First compare the subdirectories
	let subdirectories_expected: HashSet<_> = [OUTPUT_SUBDIR, KERNEL_SUBDIR, RANGE_PROOF_SUBDIR]
		.iter()
		.cloned()
		.map(|s| String::from(s))
		.collect();

	let subdirectories_found: HashSet<_> = fs::read_dir(txhashset_path)?
		.filter_map(|entry| {
			entry.ok().and_then(|e| {
				e.path()
					.file_name()
					.and_then(|n| n.to_str().map(|s| String::from(s)))
			})
		}).collect();

	let dir_difference: Vec<String> = subdirectories_found
		.difference(&subdirectories_expected)
		.cloned()
		.collect();

	// Removing unexpected directories if needed
	if !dir_difference.is_empty() {
		debug!(
			LOGGER,
			"Unexpected folder(s) found in txhashset folder, removing."
		);
		for diff in dir_difference {
			let diff_path = txhashset_path.join(diff);
			file::delete(diff_path)?;
		}
	}

	// Then compare the files found in the subdirectories
	let pmmr_files_expected: HashSet<_> = PMMR_FILES
		.iter()
		.cloned()
		.map(|s| {
			if s.contains("pmmr_leaf.bin") {
				format!("{}.{}", s, header.hash())
			} else {
				String::from(s)
			}
		}).collect();

	let subdirectories = fs::read_dir(txhashset_path)?;
	for subdirectory in subdirectories {
		let subdirectory_path = subdirectory?.path();
		let pmmr_files = fs::read_dir(&subdirectory_path)?;
		let pmmr_files_found: HashSet<_> = pmmr_files
			.filter_map(|entry| {
				entry.ok().and_then(|e| {
					e.path()
						.file_name()
						.and_then(|n| n.to_str().map(|s| String::from(s)))
				})
			}).collect();
		let difference: Vec<String> = pmmr_files_found
			.difference(&pmmr_files_expected)
			.cloned()
			.collect();
		if !difference.is_empty() {
			debug!(
				LOGGER,
				"Unexpected file(s) found in txhashset subfolder {:?}, removing.",
				&subdirectory_path
			);
			for diff in difference {
				let diff_path = subdirectory_path.join(diff);
				file::delete(diff_path)?;
			}
		}
	}
	Ok(())
}

/// Given a block header to rewind to and the block header at the
/// head of the current chain state, we need to calculate the positions
/// of all inputs (spent outputs) we need to "undo" during a rewind.
/// We do this by leveraging the "block_input_bitmap" cache and OR'ing
/// the set of bitmaps together for the set of blocks being rewound.
fn input_pos_to_rewind(
	commit_index: Arc<ChainStore>,
	block_header: &BlockHeader,
	head_header: &BlockHeader,
	batch: &Batch,
) -> Result<Bitmap, Error> {
	let mut current = head_header.hash();
	let mut height = head_header.height;

	if head_header.height < block_header.height {
		debug!(
			LOGGER,
			"input_pos_to_rewind: {} < {}, nothing to rewind",
			head_header.height,
			block_header.height
		);
		return Ok(Bitmap::create());
	}

	// Batching up the block input bitmaps, and running fast_or() on every batch of 256 bitmaps.
	// so to avoid maintaining a huge vec of bitmaps.
	let bitmap_fast_or = |b_res, block_input_bitmaps: &mut Vec<Bitmap>| -> Option<Bitmap> {
		if let Some(b) = b_res {
			block_input_bitmaps.push(b);
			if block_input_bitmaps.len() < 256 {
				return None;
			}
		}
		let bitmap = Bitmap::fast_or(&block_input_bitmaps.iter().collect::<Vec<&Bitmap>>());
		block_input_bitmaps.clear();
		block_input_bitmaps.push(bitmap.clone());
		Some(bitmap)
	};

	let mut block_input_bitmaps: Vec<Bitmap> = vec![];
	let bh = block_header.hash();

	while current != bh {
		// We cache recent block headers and block_input_bitmaps
		// internally in our db layer (commit_index).
		// I/O should be minimized or eliminated here for most
		// rewind scenarios.
		if let Ok(b_res) = batch.get_block_input_bitmap(&current) {
			bitmap_fast_or(Some(b_res), &mut block_input_bitmaps);
		}
		if height == 0 {
			break;
		}
		height -= 1;
		current = commit_index.get_hash_by_height(height)?;
	}

	let bitmap = bitmap_fast_or(None, &mut block_input_bitmaps).unwrap();
	Ok(bitmap)
}
