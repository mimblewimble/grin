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

//! Utility structs to handle the 3 hashtrees (output, range proof, kernel) more
//! conveniently and transactionally.

use std::fs;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use util::static_secp_instance;
use util::secp::pedersen::{Commitment, RangeProof};

use core::consensus::REWARD;
use core::core::{Block, BlockHeader, Input, Output, OutputFeatures, OutputIdentifier, TxKernel};
use core::core::pmmr::{self, MerkleProof, PMMR};
use core::global;
use core::core::hash::{Hash, Hashed};
use core::ser::{PMMRIndexHashable, PMMRable};

use grin_store;
use grin_store::pmmr::PMMRBackend;
use grin_store::types::prune_noop;
use keychain::BlindingFactor;
use types::{BlockMarker, ChainStore, Error, TxHashSetRoots};
use util::{zip, LOGGER};

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
	fn new(root_dir: String, file_name: &str) -> Result<PMMRHandle<T>, Error> {
		let path = Path::new(&root_dir).join(TXHASHSET_SUBDIR).join(file_name);
		fs::create_dir_all(path.clone())?;
		let be = PMMRBackend::new(path.to_str().unwrap().to_string())?;
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
	pub fn open(root_dir: String, commit_index: Arc<ChainStore>) -> Result<TxHashSet, Error> {
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
			output_pmmr_h: PMMRHandle::new(root_dir.clone(), OUTPUT_SUBDIR)?,
			rproof_pmmr_h: PMMRHandle::new(root_dir.clone(), RANGE_PROOF_SUBDIR)?,
			kernel_pmmr_h: PMMRHandle::new(root_dir.clone(), KERNEL_SUBDIR)?,
			commit_index,
		})
	}

	/// Check if an output is unspent.
	/// We look in the index to find the output MMR pos.
	/// Then we check the entry in the output MMR and confirm the hash matches.
	pub fn is_unspent(&mut self, output_id: &OutputIdentifier) -> Result<Hash, Error> {
		match self.commit_index.get_output_pos(&output_id.commit) {
			Ok(pos) => {
				let output_pmmr: PMMR<OutputIdentifier, _> =
					PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
				if let Some(hash) = output_pmmr.get_hash(pos) {
					if hash == output_id.hash_with_index(pos - 1) {
						Ok(hash)
					} else {
						Err(Error::TxHashSetErr(format!("txhashset hash mismatch")))
					}
				} else {
					Err(Error::OutputNotFound)
				}
			}
			Err(grin_store::Error::NotFoundErr) => Err(Error::OutputNotFound),
			Err(e) => Err(Error::StoreErr(e, format!("txhashset unspent check"))),
		}
	}

	/// returns the last N nodes inserted into the tree (i.e. the 'bottom'
	/// nodes at level 0
	/// TODO: These need to return the actual data from the flat-files instead of hashes now
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

	/// returns outputs from the given insertion (leaf) index up to the specified
	/// limit. Also returns the last index actually populated
	pub fn outputs_by_insertion_index(
		&mut self,
		start_index: u64,
		max_count: u64,
	) -> (u64, Vec<OutputIdentifier>) {
		let output_pmmr: PMMR<OutputIdentifier, _> =
			PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
		output_pmmr.elements_from_insertion_index(start_index, max_count)
	}

	/// highest output insertion index availalbe
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

	/// Output and kernel MMR indexes at the end of the provided block
	pub fn indexes_at(&self, bh: &Hash) -> Result<BlockMarker, Error> {
		self.commit_index.get_block_marker(bh).map_err(&From::from)
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
		let head = commit_index.head()?;
		let current_height = head.height;

		// horizon for compacting is based on current_height
		let horizon = (current_height as u32).saturating_sub(global::cut_through_horizon());

		let clean_output_index = |commit: &[u8]| {
			// do we care if this fails?
			let _ = commit_index.delete_output_pos(commit);
		};

		let min_rm = (horizon / 10) as usize;

		self.output_pmmr_h
			.backend
			.check_compact(min_rm, horizon, clean_output_index)?;

		self.rproof_pmmr_h
			.backend
			.check_compact(min_rm, horizon, &prune_noop)?;
		Ok(())
	}
}

/// Starts a new unit of work to extend (or rewind) the chain with additional blocks.
/// Accepts a closure that will operate within that unit of work.
/// The closure has access to an Extension object that allows the addition
/// of blocks to the txhashset and the checking of the current tree roots.
///
/// The unit of work is always discarded (always rollback) as this is read-only.
pub fn extending_readonly<'a, F, T>(trees: &'a mut TxHashSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&mut Extension) -> Result<T, Error>,
{
	let sizes: (u64, u64, u64);
	let res: Result<T, Error>;
	{
		let commit_index = trees.commit_index.clone();

		trace!(LOGGER, "Starting new txhashset (readonly) extension.");
		let mut extension = Extension::new(trees, commit_index);
		res = inner(&mut extension);

		sizes = extension.sizes();
	}

	debug!(
		LOGGER,
		"Rollbacking txhashset (readonly) extension. sizes {:?}", sizes
	);

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
pub fn extending<'a, F, T>(trees: &'a mut TxHashSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&mut Extension) -> Result<T, Error>,
{
	let sizes: (u64, u64, u64);
	let res: Result<T, Error>;
	let rollback: bool;

	{
		let commit_index = trees.commit_index.clone();

		trace!(LOGGER, "Starting new txhashset extension.");
		let mut extension = Extension::new(trees, commit_index);
		res = inner(&mut extension);

		rollback = extension.rollback;
		if res.is_ok() && !rollback {
			extension.save_indexes()?;
		}
		sizes = extension.sizes();
	}

	match res {
		Err(e) => {
			debug!(LOGGER, "Error returned, discarding txhashset extension.");
			trees.output_pmmr_h.backend.discard();
			trees.rproof_pmmr_h.backend.discard();
			trees.kernel_pmmr_h.backend.discard();
			Err(e)
		}
		Ok(r) => {
			if rollback {
				debug!(LOGGER, "Rollbacking txhashset extension. sizes {:?}", sizes);
				trees.output_pmmr_h.backend.discard();
				trees.rproof_pmmr_h.backend.discard();
				trees.kernel_pmmr_h.backend.discard();
			} else {
				debug!(LOGGER, "Committing txhashset extension. sizes {:?}", sizes);
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
	new_output_commits: HashMap<Commitment, u64>,
	new_block_markers: HashMap<Hash, BlockMarker>,
	rollback: bool,
}

impl<'a> Extension<'a> {
	// constructor
	fn new(trees: &'a mut TxHashSet, commit_index: Arc<ChainStore>) -> Extension<'a> {
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
			commit_index: commit_index,
			new_output_commits: HashMap::new(),
			new_block_markers: HashMap::new(),
			rollback: false,
		}
	}

	/// Apply a new set of blocks on top the existing sum trees. Blocks are
	/// applied in order of the provided Vec. If pruning is enabled, inputs also
	/// prune MMR data.
	pub fn apply_block(&mut self, b: &Block) -> Result<(), Error> {
		// first applying coinbase outputs. due to the construction of PMMRs the
		// last element, when its a leaf, can never be pruned as it has no parent
		// yet and it will be needed to calculate that hash. to work around this,
		// we insert coinbase outputs first to add at least one output of padding
		for out in &b.outputs {
			if out.features.contains(OutputFeatures::COINBASE_OUTPUT) {
				self.apply_output(out)?;
			}
		}

		// then doing inputs guarantees an input can't spend an output in the
		// same block, enforcing block cut-through
		for input in &b.inputs {
			self.apply_input(input, b.header.height)?;
		}

		// now all regular, non coinbase outputs
		for out in &b.outputs {
			if !out.features.contains(OutputFeatures::COINBASE_OUTPUT) {
				self.apply_output(out)?;
			}
		}

		// then applying all kernels
		for kernel in &b.kernels {
			self.apply_kernel(kernel)?;
		}

		// finally, recording the PMMR positions after this block for future rewind
		let marker = BlockMarker {
			output_pos: self.output_pmmr.unpruned_size(),
			kernel_pos: self.kernel_pmmr.unpruned_size(),
		};
		self.new_block_markers.insert(b.hash(), marker);
		Ok(())
	}

	fn save_indexes(&self) -> Result<(), Error> {
		// store all new output pos in the index
		for (commit, pos) in &self.new_output_commits {
			self.commit_index.save_output_pos(commit, *pos)?;
		}
		for (bh, marker) in &self.new_block_markers {
			self.commit_index.save_block_marker(bh, marker)?;
		}
		Ok(())
	}

	fn apply_input(&mut self, input: &Input, height: u64) -> Result<(), Error> {
		let commit = input.commitment();
		let pos_res = self.get_output_pos(&commit);
		if let Ok(pos) = pos_res {
			let output_id_hash = OutputIdentifier::from_input(input).hash_with_index(pos - 1);
			if let Some(read_hash) = self.output_pmmr.get_hash(pos) {
				// check hash from pmmr matches hash from input (or corresponding output)
				// if not then the input is not being honest about
				// what it is attempting to spend...

				let read_elem = self.output_pmmr.get_data(pos);

				if output_id_hash != read_hash
					|| output_id_hash
						!= read_elem
							.expect("no output at position")
							.hash_with_index(pos - 1)
				{
					return Err(Error::TxHashSetErr(format!("output pmmr hash mismatch")));
				}

				// check coinbase maturity with the Merkle Proof on the input
				if input.features.contains(OutputFeatures::COINBASE_OUTPUT) {
					let header = self.commit_index.get_block_header(&input.block_hash())?;
					input.verify_maturity(read_hash, &header, height)?;
				}
			}

			// Now prune the output_pmmr, rproof_pmmr and their storage.
			// Input is not valid if we cannot prune successfully (to spend an unspent
			// output).
			match self.output_pmmr.prune(pos, height as u32) {
				Ok(true) => {
					self.rproof_pmmr
						.prune(pos, height as u32)
						.map_err(|s| Error::TxHashSetErr(s))?;
				}
				Ok(false) => return Err(Error::AlreadySpent(commit)),
				Err(s) => return Err(Error::TxHashSetErr(s)),
			}
		} else {
			return Err(Error::AlreadySpent(commit));
		}
		Ok(())
	}

	fn apply_output(&mut self, out: &Output) -> Result<(), Error> {
		let commit = out.commitment();

		if let Ok(pos) = self.get_output_pos(&commit) {
			// we need to check whether the commitment is in the current MMR view
			// as well as the index doesn't support rewind and is non-authoritative
			// (non-historical node will have a much smaller one)
			// note that this doesn't show the commitment *never* existed, just
			// that this is not an existing unspent commitment right now
			if let Some(hash) = self.output_pmmr.get_hash(pos) {
				// processing a new fork so we may get a position on the old
				// fork that exists but matches a different node
				// filtering that case out
				if hash == OutputIdentifier::from_output(out).hash() {
					return Err(Error::DuplicateCommitment(commit));
				}
			}
		}
		// push new outputs in their MMR and save them in the index
		let pos = self.output_pmmr
			.push(OutputIdentifier::from_output(out))
			.map_err(&Error::TxHashSetErr)?;
		self.new_output_commits.insert(out.commitment(), pos);

		// push range proofs in their MMR and file
		self.rproof_pmmr
			.push(out.proof)
			.map_err(&Error::TxHashSetErr)?;
		Ok(())
	}

	fn apply_kernel(&mut self, kernel: &TxKernel) -> Result<(), Error> {
		// push kernels in their MMR and file
		self.kernel_pmmr
			.push(kernel.clone())
			.map_err(&Error::TxHashSetErr)?;

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
		let pos = self.get_output_pos(&output.commit)?;
		let merkle_proof = self.output_pmmr
			.merkle_proof(pos)
			.map_err(&Error::TxHashSetErr)?;

		Ok(merkle_proof)
	}

	/// Rewinds the MMRs to the provided block, using the last output and
	/// last kernel of the block we want to rewind to.
	pub fn rewind(&mut self, block_header: &BlockHeader) -> Result<(), Error> {
		let hash = block_header.hash();
		let height = block_header.height;
		debug!(LOGGER, "Rewind to header {} @ {}", height, hash);

		// rewind our MMRs to the appropriate pos
		// based on block height and block marker
		let marker = self.commit_index.get_block_marker(&hash)?;
		self.rewind_to_marker(height, &marker)?;
		Ok(())
	}

	/// Rewinds the MMRs to the provided positions, given the output and
	/// kernel we want to rewind to.
	fn rewind_to_marker(&mut self, height: u64, marker: &BlockMarker) -> Result<(), Error> {
		debug!(LOGGER, "Rewind txhashset to {}, {:?}", height, marker);

		self.output_pmmr
			.rewind(marker.output_pos, height as u32)
			.map_err(&Error::TxHashSetErr)?;
		self.rproof_pmmr
			.rewind(marker.output_pos, height as u32)
			.map_err(&Error::TxHashSetErr)?;
		self.kernel_pmmr
			.rewind(marker.kernel_pos, height as u32)
			.map_err(&Error::TxHashSetErr)?;

		Ok(())
	}

	fn get_output_pos(&self, commit: &Commitment) -> Result<u64, grin_store::Error> {
		if let Some(pos) = self.new_output_commits.get(commit) {
			Ok(*pos)
		} else {
			self.commit_index.get_output_pos(commit)
		}
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
		// If we are validating the genesis block then
		// we have no outputs or kernels.
		// So we are done here.
		if header.height == 0 {
			return Ok(());
		}

		let roots = self.roots();
		if roots.output_root != header.output_root || roots.rproof_root != header.range_proof_root
			|| roots.kernel_root != header.kernel_root
		{
			return Err(Error::InvalidRoot);
		}
		Ok(())
	}

	/// Validate the txhashset state against the provided block header.
	pub fn validate(&mut self, header: &BlockHeader, skip_rproofs: bool) -> Result<(), Error> {
		// validate all hashes and sums within the trees
		if let Err(e) = self.output_pmmr.validate() {
			return Err(Error::InvalidTxHashSet(e));
		}
		if let Err(e) = self.rproof_pmmr.validate() {
			return Err(Error::InvalidTxHashSet(e));
		}
		if let Err(e) = self.kernel_pmmr.validate() {
			return Err(Error::InvalidTxHashSet(e));
		}

		self.validate_roots(header)?;

		if header.height == 0 {
			return Ok(());
		}

		// the real magicking: the sum of all kernel excess should equal the sum
		// of all Output commitments, minus the total supply
		let kernel_offset = self.sum_kernel_offsets(&header)?;
		let kernel_sum = self.sum_kernels(kernel_offset)?;
		let output_sum = self.sum_outputs()?;

		// supply is the sum of the coinbase outputs from all the block headers
		let supply = header.height * REWARD;

		{
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();

			let over_commit = secp.commit_value(supply)?;
			let adjusted_sum_output = secp.commit_sum(vec![output_sum], vec![over_commit])?;
			if adjusted_sum_output != kernel_sum {
				return Err(Error::InvalidTxHashSet(
					"Differing Output commitment and kernel excess sums.".to_owned(),
				));
			}
		}

		// now verify the rangeproof for each output in the sum above
		// this is an expensive operation (only verified if requested)
		if !skip_rproofs {
			self.verify_rangeproofs()?;
		}

		Ok(())
	}

	/// Rebuild the index of MMR positions to the corresponding Output and kernel
	/// by iterating over the whole MMR data. This is a costly operation
	/// performed only when we receive a full new chain state.
	pub fn rebuild_index(&self) -> Result<(), Error> {
		for n in 1..self.output_pmmr.unpruned_size() + 1 {
			// non-pruned leaves only
			if pmmr::bintree_postorder_height(n) == 0 {
				if let Some(out) = self.output_pmmr.get_data(n) {
					self.commit_index.save_output_pos(&out.commit, n)?;
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

	// Sizes of the sum trees, used by `extending` on rollback.
	fn sizes(&self) -> (u64, u64, u64) {
		(
			self.output_pmmr.unpruned_size(),
			self.rproof_pmmr.unpruned_size(),
			self.kernel_pmmr.unpruned_size(),
		)
	}

	// We maintain the total accumulated kernel offset in each block header.
	// So "summing" is just a case of taking the total kernel offset
	// directly from the current block header.
	fn sum_kernel_offsets(&self, header: &BlockHeader) -> Result<Option<Commitment>, Error> {
		let offset = if header.total_kernel_offset == BlindingFactor::zero() {
			None
		} else {
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			let skey = header.total_kernel_offset.secret_key(&secp)?;
			Some(secp.commit(0, skey)?)
		};
		Ok(offset)
	}

	/// Sums the excess of all our kernels, validating their signatures on the
	/// way
	fn sum_kernels(&self, kernel_offset: Option<Commitment>) -> Result<Commitment, Error> {
		let now = Instant::now();

		let mut commitments = vec![];
		if let Some(offset) = kernel_offset {
			commitments.push(offset);
		}

		for n in 1..self.kernel_pmmr.unpruned_size() + 1 {
			if pmmr::is_leaf(n) {
				if let Some(kernel) = self.kernel_pmmr.get_data(n) {
					kernel.verify()?;
					commitments.push(kernel.excess);
				}
			}
		}

		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		let kern_count = commitments.len();
		let sum_kernel = secp.commit_sum(commitments, vec![])?;

		debug!(
			LOGGER,
			"Validated, summed (and offset) {} kernels, pmmr size {}, took {}s",
			kern_count,
			self.kernel_pmmr.unpruned_size(),
			now.elapsed().as_secs(),
		);
		Ok(sum_kernel)
	}

	fn verify_rangeproofs(&self) -> Result<(), Error> {
		let now = Instant::now();

		let mut proof_count = 0;
		for n in 1..self.output_pmmr.unpruned_size() + 1 {
			if pmmr::is_leaf(n) {
				if let Some(out) = self.output_pmmr.get_data(n) {
					if let Some(rp) = self.rproof_pmmr.get_data(n) {
						out.to_output(rp).verify_proof()?;
					} else {
						// TODO - rangeproof not found
						return Err(Error::OutputNotFound);
					}
					proof_count += 1;

					if proof_count % 500 == 0 {
						debug!(
							LOGGER,
							"txhashset: verify_rangeproofs: verified {} rangeproofs", proof_count,
						);
					}
				}
			}
		}
		debug!(
			LOGGER,
			"Verified {} Rangeproofs, pmmr size {}, took {}s",
			proof_count,
			self.rproof_pmmr.unpruned_size(),
			now.elapsed().as_secs(),
		);
		Ok(())
	}

	/// Sums all our Output commitments, checking range proofs at the same time
	fn sum_outputs(&self) -> Result<Commitment, Error> {
		let now = Instant::now();

		let mut commitments = vec![];
		for n in 1..self.output_pmmr.unpruned_size() + 1 {
			if pmmr::is_leaf(n) {
				if let Some(out) = self.output_pmmr.get_data(n) {
					commitments.push(out.commit);
				}
			}
		}

		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		let commit_count = commitments.len();
		let sum_output = secp.commit_sum(commitments, vec![])?;

		debug!(
			LOGGER,
			"Summed {} Outputs, pmmr size {}, took {}s",
			commit_count,
			self.output_pmmr.unpruned_size(),
			now.elapsed().as_secs(),
		);

		Ok(sum_output)
	}
}

/// Packages the txhashset data files into a zip and returns a Read to the
/// resulting file
pub fn zip_read(root_dir: String) -> Result<File, Error> {
	let txhashset_path = Path::new(&root_dir).join(TXHASHSET_SUBDIR);
	let zip_path = Path::new(&root_dir).join(TXHASHSET_ZIP);

	// create the zip archive
	{
		zip::compress(&txhashset_path, &File::create(zip_path.clone())?)
			.map_err(|ze| Error::Other(ze.to_string()))?;
	}

	// open it again to read it back
	let zip_file = File::open(zip_path)?;
	Ok(zip_file)
}

/// Extract the txhashset data from a zip file and writes the content into the
/// txhashset storage dir
pub fn zip_write(root_dir: String, txhashset_data: File) -> Result<(), Error> {
	let txhashset_path = Path::new(&root_dir).join(TXHASHSET_SUBDIR);

	fs::create_dir_all(txhashset_path.clone())?;
	zip::decompress(txhashset_data, &txhashset_path).map_err(|ze| Error::Other(ze.to_string()))
}
