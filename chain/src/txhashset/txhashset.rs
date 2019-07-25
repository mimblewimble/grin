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

//! Utility structs to handle the 3 MMRs (output, rangeproof,
//! kernel) along the overall header MMR conveniently and transactionally.

use crate::core::core::committed::Committed;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::merkle_proof::MerkleProof;
use crate::core::core::pmmr::{self, Backend, ReadonlyPMMR, RewindablePMMR, PMMR};
use crate::core::core::{
	Block, BlockHeader, Input, Output, OutputIdentifier, TxKernel, TxKernelEntry,
};
use crate::core::global;
use crate::core::ser::{PMMRIndexHashable, PMMRable};
use crate::error::{Error, ErrorKind};
use crate::store::{Batch, ChainStore};
use crate::txhashset::{RewindableKernelView, UTXOView};
use crate::types::{Tip, TxHashSetRoots, TxHashsetWriteStatus};
use crate::util::secp::pedersen::{Commitment, RangeProof};
use crate::util::{file, secp_static, zip};
use croaring::Bitmap;
use grin_store;
use grin_store::pmmr::{clean_files_by_prefix, PMMRBackend};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const HEADERHASHSET_SUBDIR: &'static str = "header";
const TXHASHSET_SUBDIR: &'static str = "txhashset";

const HEADER_HEAD_SUBDIR: &'static str = "header_head";
const SYNC_HEAD_SUBDIR: &'static str = "sync_head";

const OUTPUT_SUBDIR: &'static str = "output";
const RANGE_PROOF_SUBDIR: &'static str = "rangeproof";
const KERNEL_SUBDIR: &'static str = "kernel";

const TXHASHSET_ZIP: &'static str = "txhashset_snapshot";

struct PMMRHandle<T: PMMRable> {
	backend: PMMRBackend<T>,
	last_pos: u64,
}

impl<T: PMMRable> PMMRHandle<T> {
	fn new(
		root_dir: &str,
		sub_dir: &str,
		file_name: &str,
		prunable: bool,
		fixed_size: bool,
		header: Option<&BlockHeader>,
	) -> Result<PMMRHandle<T>, Error> {
		let path = Path::new(root_dir).join(sub_dir).join(file_name);
		fs::create_dir_all(path.clone())?;
		let path_str = path.to_str().ok_or(Error::from(ErrorKind::Other(
			"invalid file path".to_owned(),
		)))?;
		let backend = PMMRBackend::new(path_str.to_string(), prunable, fixed_size, header)?;
		let last_pos = backend.unpruned_size();
		Ok(PMMRHandle { backend, last_pos })
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
	/// Header MMR to support the header_head chain.
	/// This is rewound and applied transactionally with the
	/// output, rangeproof and kernel MMRs during an extension or a
	/// readonly_extension.
	/// It can also be rewound and applied separately via a header_extension.
	header_pmmr_h: PMMRHandle<BlockHeader>,

	/// Header MMR to support exploratory sync_head.
	/// The header_head and sync_head chains can diverge so we need to maintain
	/// multiple header MMRs during the sync process.
	///
	/// Note: this is rewound and applied separately to the other MMRs
	/// via a "sync_extension".
	sync_pmmr_h: PMMRHandle<BlockHeader>,

	output_pmmr_h: PMMRHandle<Output>,
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
		Ok(TxHashSet {
			header_pmmr_h: PMMRHandle::new(
				&root_dir,
				HEADERHASHSET_SUBDIR,
				HEADER_HEAD_SUBDIR,
				false,
				true,
				None,
			)?,
			sync_pmmr_h: PMMRHandle::new(
				&root_dir,
				HEADERHASHSET_SUBDIR,
				SYNC_HEAD_SUBDIR,
				false,
				true,
				None,
			)?,
			output_pmmr_h: PMMRHandle::new(
				&root_dir,
				TXHASHSET_SUBDIR,
				OUTPUT_SUBDIR,
				true,
				true,
				header,
			)?,
			rproof_pmmr_h: PMMRHandle::new(
				&root_dir,
				TXHASHSET_SUBDIR,
				RANGE_PROOF_SUBDIR,
				true,
				true,
				header,
			)?,
			kernel_pmmr_h: PMMRHandle::new(
				&root_dir,
				TXHASHSET_SUBDIR,
				KERNEL_SUBDIR,
				false, // not prunable
				false, // variable size kernel data file
				None,
			)?,
			commit_index,
		})
	}

	/// Close all backend file handles
	pub fn release_backend_files(&mut self) {
		self.header_pmmr_h.backend.release_files();
		self.sync_pmmr_h.backend.release_files();
		self.output_pmmr_h.backend.release_files();
		self.rproof_pmmr_h.backend.release_files();
		self.kernel_pmmr_h.backend.release_files();
	}

	/// Check if an output is unspent.
	/// We look in the index to find the output MMR pos.
	/// Then we check the entry in the output MMR and confirm the hash matches.
	pub fn is_unspent(&self, output_id: &OutputIdentifier) -> Result<(Hash, u64), Error> {
		match self.commit_index.get_output_pos(&output_id.commit) {
			Ok(pos) => {
				let output_pmmr: ReadonlyPMMR<'_, Output, _> =
					ReadonlyPMMR::at(&self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
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
	pub fn last_n_output(&self, distance: u64) -> Vec<(Hash, OutputIdentifier)> {
		ReadonlyPMMR::at(&self.output_pmmr_h.backend, self.output_pmmr_h.last_pos)
			.get_last_n_insertions(distance)
	}

	/// as above, for range proofs
	pub fn last_n_rangeproof(&self, distance: u64) -> Vec<(Hash, RangeProof)> {
		ReadonlyPMMR::at(&self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos)
			.get_last_n_insertions(distance)
	}

	/// as above, for kernels
	pub fn last_n_kernel(&self, distance: u64) -> Vec<(Hash, TxKernelEntry)> {
		ReadonlyPMMR::at(&self.kernel_pmmr_h.backend, self.kernel_pmmr_h.last_pos)
			.get_last_n_insertions(distance)
	}

	/// Get the header hash at the specified height based on the current state of the txhashset.
	pub fn get_header_hash_by_height(&self, height: u64) -> Result<Hash, Error> {
		let pos = pmmr::insertion_to_pmmr_index(height + 1);
		let header_pmmr =
			ReadonlyPMMR::at(&self.header_pmmr_h.backend, self.header_pmmr_h.last_pos);
		if let Some(entry) = header_pmmr.get_data(pos) {
			Ok(entry.hash())
		} else {
			Err(ErrorKind::Other(format!("get header hash by height")).into())
		}
	}

	/// Get the header at the specified height based on the current state of the txhashset.
	/// Derives the MMR pos from the height (insertion index) and retrieves the header hash.
	/// Looks the header up in the db by hash.
	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		let hash = self.get_header_hash_by_height(height)?;
		let header = self.commit_index.get_block_header(&hash)?;
		Ok(header)
	}

	/// returns outputs from the given insertion (leaf) index up to the
	/// specified limit. Also returns the last index actually populated
	pub fn outputs_by_insertion_index(
		&self,
		start_index: u64,
		max_count: u64,
	) -> (u64, Vec<OutputIdentifier>) {
		ReadonlyPMMR::at(&self.output_pmmr_h.backend, self.output_pmmr_h.last_pos)
			.elements_from_insertion_index(start_index, max_count)
	}

	/// highest output insertion index available
	pub fn highest_output_insertion_index(&self) -> u64 {
		pmmr::n_leaves(self.output_pmmr_h.last_pos)
	}

	/// As above, for rangeproofs
	pub fn rangeproofs_by_insertion_index(
		&self,
		start_index: u64,
		max_count: u64,
	) -> (u64, Vec<RangeProof>) {
		ReadonlyPMMR::at(&self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos)
			.elements_from_insertion_index(start_index, max_count)
	}

	/// Get MMR roots.
	pub fn roots(&self) -> TxHashSetRoots {
		let header_pmmr =
			ReadonlyPMMR::at(&self.header_pmmr_h.backend, self.header_pmmr_h.last_pos);
		let output_pmmr =
			ReadonlyPMMR::at(&self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
		let rproof_pmmr =
			ReadonlyPMMR::at(&self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos);
		let kernel_pmmr =
			ReadonlyPMMR::at(&self.kernel_pmmr_h.backend, self.kernel_pmmr_h.last_pos);

		TxHashSetRoots {
			header_root: header_pmmr.root(),
			output_root: output_pmmr.root(),
			rproof_root: rproof_pmmr.root(),
			kernel_root: kernel_pmmr.root(),
		}
	}

	/// Return Commit's MMR position
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		Ok(self.commit_index.get_output_pos(&commit)?)
	}

	/// build a new merkle proof for the given position.
	pub fn merkle_proof(&mut self, commit: Commitment) -> Result<MerkleProof, Error> {
		let pos = self.commit_index.get_output_pos(&commit)?;
		PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos)
			.merkle_proof(pos)
			.map_err(|_| ErrorKind::MerkleProof.into())
	}

	/// Compact the MMR data files and flush the rm logs
	pub fn compact(&mut self, batch: &mut Batch<'_>) -> Result<(), Error> {
		debug!("txhashset: starting compaction...");

		let head_header = batch.head_header()?;
		let current_height = head_header.height;

		// horizon for compacting is based on current_height
		let horizon_height = current_height.saturating_sub(global::cut_through_horizon().into());
		let horizon_hash = self.get_header_hash_by_height(horizon_height)?;
		let horizon_header = batch.get_block_header(&horizon_hash)?;

		let rewind_rm_pos = input_pos_to_rewind(&horizon_header, &head_header, batch)?;

		debug!("txhashset: check_compact output mmr backend...");
		self.output_pmmr_h
			.backend
			.check_compact(horizon_header.output_mmr_size, &rewind_rm_pos)?;

		debug!("txhashset: check_compact rangeproof mmr backend...");
		self.rproof_pmmr_h
			.backend
			.check_compact(horizon_header.output_mmr_size, &rewind_rm_pos)?;

		debug!("txhashset: ... compaction finished");

		Ok(())
	}
}

/// Starts a new unit of work to extend (or rewind) the chain with additional
/// blocks. Accepts a closure that will operate within that unit of work.
/// The closure has access to an Extension object that allows the addition
/// of blocks to the txhashset and the checking of the current tree roots.
///
/// The unit of work is always discarded (always rollback) as this is read-only.
pub fn extending_readonly<F, T>(trees: &mut TxHashSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&mut Extension<'_>) -> Result<T, Error>,
{
	let commit_index = trees.commit_index.clone();
	let batch = commit_index.batch()?;

	// We want to use the current head of the most work chain unless
	// we explicitly rewind the extension.
	let head = batch.head()?;

	trace!("Starting new txhashset (readonly) extension.");

	let res = {
		let mut extension = Extension::new(trees, &batch, head);
		extension.force_rollback();
		inner(&mut extension)
	};

	trace!("Rollbacking txhashset (readonly) extension.");

	trees.header_pmmr_h.backend.discard();
	trees.output_pmmr_h.backend.discard();
	trees.rproof_pmmr_h.backend.discard();
	trees.kernel_pmmr_h.backend.discard();

	trace!("TxHashSet (readonly) extension done.");

	res
}

/// Readonly view on the UTXO set.
/// Based on the current txhashset output_pmmr.
pub fn utxo_view<F, T>(trees: &TxHashSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&UTXOView<'_>) -> Result<T, Error>,
{
	let res: Result<T, Error>;
	{
		let output_pmmr =
			ReadonlyPMMR::at(&trees.output_pmmr_h.backend, trees.output_pmmr_h.last_pos);
		let header_pmmr =
			ReadonlyPMMR::at(&trees.header_pmmr_h.backend, trees.header_pmmr_h.last_pos);

		// Create a new batch here to pass into the utxo_view.
		// Discard it (rollback) after we finish with the utxo_view.
		let batch = trees.commit_index.batch()?;
		let utxo = UTXOView::new(output_pmmr, header_pmmr, &batch);
		res = inner(&utxo);
	}
	res
}

/// Rewindable (but still readonly) view on the kernel MMR.
/// The underlying backend is readonly. But we permit the PMMR to be "rewound"
/// via last_pos.
/// We create a new db batch for this view and discard it (rollback)
/// when we are done with the view.
pub fn rewindable_kernel_view<F, T>(trees: &TxHashSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&mut RewindableKernelView<'_>) -> Result<T, Error>,
{
	let res: Result<T, Error>;
	{
		let kernel_pmmr =
			RewindablePMMR::at(&trees.kernel_pmmr_h.backend, trees.kernel_pmmr_h.last_pos);

		// Create a new batch here to pass into the kernel_view.
		// Discard it (rollback) after we finish with the kernel_view.
		let batch = trees.commit_index.batch()?;
		let header = batch.head_header()?;
		let mut view = RewindableKernelView::new(kernel_pmmr, &batch, header);
		res = inner(&mut view);
	}
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
	batch: &'a mut Batch<'_>,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut Extension<'_>) -> Result<T, Error>,
{
	let sizes: (u64, u64, u64, u64);
	let res: Result<T, Error>;
	let rollback: bool;

	// We want to use the current head of the most work chain unless
	// we explicitly rewind the extension.
	let head = batch.head()?;

	// create a child transaction so if the state is rolled back by itself, all
	// index saving can be undone
	let child_batch = batch.child()?;
	{
		trace!("Starting new txhashset extension.");

		let mut extension = Extension::new(trees, &child_batch, head);
		res = inner(&mut extension);

		rollback = extension.rollback;
		sizes = extension.sizes();
	}

	match res {
		Err(e) => {
			debug!("Error returned, discarding txhashset extension: {}", e);
			trees.header_pmmr_h.backend.discard();
			trees.output_pmmr_h.backend.discard();
			trees.rproof_pmmr_h.backend.discard();
			trees.kernel_pmmr_h.backend.discard();
			Err(e)
		}
		Ok(r) => {
			if rollback {
				trace!("Rollbacking txhashset extension. sizes {:?}", sizes);
				trees.header_pmmr_h.backend.discard();
				trees.output_pmmr_h.backend.discard();
				trees.rproof_pmmr_h.backend.discard();
				trees.kernel_pmmr_h.backend.discard();
			} else {
				trace!("Committing txhashset extension. sizes {:?}", sizes);
				child_batch.commit()?;
				// NOTE: The header MMR is readonly for a txhashset extension.
				trees.header_pmmr_h.backend.discard();
				trees.output_pmmr_h.backend.sync()?;
				trees.rproof_pmmr_h.backend.sync()?;
				trees.kernel_pmmr_h.backend.sync()?;
				trees.output_pmmr_h.last_pos = sizes.1;
				trees.rproof_pmmr_h.last_pos = sizes.2;
				trees.kernel_pmmr_h.last_pos = sizes.3;
			}

			trace!("TxHashSet extension done.");
			Ok(r)
		}
	}
}

/// Start a new sync MMR unit of work. This MMR tracks the sync_head.
/// This is used during header sync to validate batches of headers as they arrive
/// without needing to repeatedly rewind the header MMR that continues to track
/// the header_head as they diverge during sync.
pub fn sync_extending<'a, F, T>(
	trees: &'a mut TxHashSet,
	batch: &'a mut Batch<'_>,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut HeaderExtension<'_>) -> Result<T, Error>,
{
	let size: u64;
	let res: Result<T, Error>;
	let rollback: bool;

	// We want to use the current sync_head unless
	// we explicitly rewind the extension.
	let head = batch.get_sync_head()?;

	// create a child transaction so if the state is rolled back by itself, all
	// index saving can be undone
	let child_batch = batch.child()?;
	{
		trace!("Starting new txhashset sync_head extension.");
		let pmmr = PMMR::at(&mut trees.sync_pmmr_h.backend, trees.sync_pmmr_h.last_pos);
		let mut extension = HeaderExtension::new(pmmr, &child_batch, head);

		res = inner(&mut extension);

		rollback = extension.rollback;
		size = extension.size();
	}

	match res {
		Err(e) => {
			debug!(
				"Error returned, discarding txhashset sync_head extension: {}",
				e
			);
			trees.sync_pmmr_h.backend.discard();
			Err(e)
		}
		Ok(r) => {
			if rollback {
				trace!("Rollbacking txhashset sync_head extension. size {:?}", size);
				trees.sync_pmmr_h.backend.discard();
			} else {
				trace!("Committing txhashset sync_head extension. size {:?}", size);
				child_batch.commit()?;
				trees.sync_pmmr_h.backend.sync()?;
				trees.sync_pmmr_h.last_pos = size;
			}
			trace!("TxHashSet sync_head extension done.");
			Ok(r)
		}
	}
}

/// Start a new header MMR unit of work. This MMR tracks the header_head.
/// This MMR can be extended individually beyond the other (output, rangeproof and kernel) MMRs
/// to allow headers to be validated before we receive the full block data.
pub fn header_extending<'a, F, T>(
	trees: &'a mut TxHashSet,
	batch: &'a mut Batch<'_>,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut HeaderExtension<'_>) -> Result<T, Error>,
{
	let size: u64;
	let res: Result<T, Error>;
	let rollback: bool;

	// We want to use the current head of the most work chain unless
	// we explicitly rewind the extension.
	let head = batch.head()?;

	// create a child transaction so if the state is rolled back by itself, all
	// index saving can be undone
	let child_batch = batch.child()?;
	{
		trace!("Starting new txhashset header extension.");
		let pmmr = PMMR::at(
			&mut trees.header_pmmr_h.backend,
			trees.header_pmmr_h.last_pos,
		);
		let mut extension = HeaderExtension::new(pmmr, &child_batch, head);
		res = inner(&mut extension);

		rollback = extension.rollback;
		size = extension.size();
	}

	match res {
		Err(e) => {
			debug!(
				"Error returned, discarding txhashset header extension: {}",
				e
			);
			trees.header_pmmr_h.backend.discard();
			Err(e)
		}
		Ok(r) => {
			if rollback {
				trace!("Rollbacking txhashset header extension. size {:?}", size);
				trees.header_pmmr_h.backend.discard();
			} else {
				trace!("Committing txhashset header extension. size {:?}", size);
				child_batch.commit()?;
				trees.header_pmmr_h.backend.sync()?;
				trees.header_pmmr_h.last_pos = size;
			}
			trace!("TxHashSet header extension done.");
			Ok(r)
		}
	}
}

/// A header extension to allow the header MMR to extend beyond the other MMRs individually.
/// This is to allow headers to be validated against the MMR before we have the full block data.
pub struct HeaderExtension<'a> {
	head: Tip,

	pmmr: PMMR<'a, BlockHeader, PMMRBackend<BlockHeader>>,

	/// Rollback flag.
	rollback: bool,

	/// Batch in which the extension occurs, public so it can be used within
	/// an `extending` closure. Just be careful using it that way as it will
	/// get rolled back with the extension (i.e on a losing fork).
	pub batch: &'a Batch<'a>,
}

impl<'a> HeaderExtension<'a> {
	fn new(
		pmmr: PMMR<'a, BlockHeader, PMMRBackend<BlockHeader>>,
		batch: &'a Batch<'_>,
		head: Tip,
	) -> HeaderExtension<'a> {
		HeaderExtension {
			head,
			pmmr,
			rollback: false,
			batch,
		}
	}

	/// Get the header hash for the specified pos from the underlying MMR backend.
	fn get_header_hash(&self, pos: u64) -> Option<Hash> {
		self.pmmr.get_data(pos).map(|x| x.hash())
	}

	/// The head representing the furthest extent of the current extension.
	pub fn head(&self) -> Tip {
		self.head.clone()
	}

	/// Get the header at the specified height based on the current state of the header extension.
	/// Derives the MMR pos from the height (insertion index) and retrieves the header hash.
	/// Looks the header up in the db by hash.
	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		let pos = pmmr::insertion_to_pmmr_index(height + 1);
		if let Some(hash) = self.get_header_hash(pos) {
			let header = self.batch.get_block_header(&hash)?;
			Ok(header)
		} else {
			Err(ErrorKind::Other(format!("get header by height")).into())
		}
	}

	/// Compares the provided header to the header in the header MMR at that height.
	/// If these match we know the header is on the current chain.
	pub fn is_on_current_chain(&self, header: &BlockHeader) -> Result<(), Error> {
		let chain_header = self.get_header_by_height(header.height)?;
		if chain_header.hash() == header.hash() {
			Ok(())
		} else {
			Err(ErrorKind::Other(format!("not on current chain")).into())
		}
	}

	/// Force the rollback of this extension, no matter the result.
	pub fn force_rollback(&mut self) {
		self.rollback = true;
	}

	/// Apply a new header to the header MMR extension.
	/// This may be either the header MMR or the sync MMR depending on the
	/// extension.
	pub fn apply_header(&mut self, header: &BlockHeader) -> Result<Hash, Error> {
		self.pmmr.push(header).map_err(&ErrorKind::TxHashSetErr)?;
		self.head = Tip::from_header(header);
		Ok(self.root())
	}

	/// Rewind the header extension to the specified header.
	/// Note the close relationship between header height and insertion index.
	pub fn rewind(&mut self, header: &BlockHeader) -> Result<(), Error> {
		debug!(
			"Rewind header extension to {} at {}",
			header.hash(),
			header.height
		);

		let header_pos = pmmr::insertion_to_pmmr_index(header.height + 1);
		self.pmmr
			.rewind(header_pos, &Bitmap::create())
			.map_err(&ErrorKind::TxHashSetErr)?;

		// Update our head to reflect the header we rewound to.
		self.head = Tip::from_header(header);

		Ok(())
	}

	/// Truncate the header MMR (rewind all the way back to pos 0).
	/// Used when rebuilding the header MMR by reapplying all headers
	/// including the genesis block header.
	pub fn truncate(&mut self) -> Result<(), Error> {
		debug!("Truncating header extension.");
		self.pmmr.truncate().map_err(&ErrorKind::TxHashSetErr)?;
		Ok(())
	}

	/// The size of the header MMR.
	pub fn size(&self) -> u64 {
		self.pmmr.unpruned_size()
	}

	/// TODO - think about how to optimize this.
	/// Requires *all* header hashes to be iterated over in ascending order.
	pub fn rebuild(&mut self, head: &Tip, genesis: &BlockHeader) -> Result<(), Error> {
		debug!(
			"About to rebuild header extension from {:?} to {:?}.",
			genesis.hash(),
			head.last_block_h,
		);

		let mut header_hashes = vec![];
		let mut current = self.batch.get_block_header(&head.last_block_h)?;
		while current.height > 0 {
			header_hashes.push(current.hash());
			current = self.batch.get_previous_header(&current)?;
		}

		header_hashes.reverse();

		// Trucate the extension (back to pos 0).
		self.truncate()?;

		// Re-apply the genesis header after truncation.
		self.apply_header(&genesis)?;

		if header_hashes.len() > 0 {
			debug!(
				"Re-applying {} headers to extension, from {:?} to {:?}.",
				header_hashes.len(),
				header_hashes.first().unwrap(),
				header_hashes.last().unwrap(),
			);

			for h in header_hashes {
				let header = self.batch.get_block_header(&h)?;
				self.validate_root(&header)?;
				self.apply_header(&header)?;
			}
		}
		Ok(())
	}

	/// The root of the header MMR for convenience.
	pub fn root(&self) -> Hash {
		self.pmmr.root()
	}

	/// Validate the prev_root of the header against the root of the current header MMR.
	pub fn validate_root(&self, header: &BlockHeader) -> Result<(), Error> {
		// If we are validating the genesis block then we have no prev_root.
		// So we are done here.
		if header.height == 0 {
			return Ok(());
		}
		if self.root() != header.prev_root {
			Err(ErrorKind::InvalidRoot.into())
		} else {
			Ok(())
		}
	}
}

/// Allows the application of new blocks on top of the sum trees in a
/// reversible manner within a unit of work provided by the `extending`
/// function.
pub struct Extension<'a> {
	head: Tip,

	header_pmmr: PMMR<'a, BlockHeader, PMMRBackend<BlockHeader>>,
	output_pmmr: PMMR<'a, Output, PMMRBackend<Output>>,
	rproof_pmmr: PMMR<'a, RangeProof, PMMRBackend<RangeProof>>,
	kernel_pmmr: PMMR<'a, TxKernel, PMMRBackend<TxKernel>>,

	/// Rollback flag.
	rollback: bool,

	/// Batch in which the extension occurs, public so it can be used within
	/// an `extending` closure. Just be careful using it that way as it will
	/// get rolled back with the extension (i.e on a losing fork).
	pub batch: &'a Batch<'a>,
}

impl<'a> Committed for Extension<'a> {
	fn inputs_committed(&self) -> Vec<Commitment> {
		vec![]
	}

	fn outputs_committed(&self) -> Vec<Commitment> {
		let mut commitments = vec![];
		for pos in self.output_pmmr.leaf_pos_iter() {
			if let Some(out) = self.output_pmmr.get_data(pos) {
				commitments.push(out.commit);
			}
		}
		commitments
	}

	fn kernels_committed(&self) -> Vec<Commitment> {
		let mut commitments = vec![];
		for n in 1..self.kernel_pmmr.unpruned_size() + 1 {
			if pmmr::is_leaf(n) {
				if let Some(kernel) = self.kernel_pmmr.get_data(n) {
					commitments.push(kernel.excess());
				}
			}
		}
		commitments
	}
}

impl<'a> Extension<'a> {
	fn new(trees: &'a mut TxHashSet, batch: &'a Batch<'_>, head: Tip) -> Extension<'a> {
		Extension {
			head,
			header_pmmr: PMMR::at(
				&mut trees.header_pmmr_h.backend,
				trees.header_pmmr_h.last_pos,
			),
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
			rollback: false,
			batch,
		}
	}

	/// The head representing the furthest extent of the current extension.
	pub fn head(&self) -> Tip {
		self.head.clone()
	}

	/// Build a view of the current UTXO set based on the output PMMR.
	pub fn utxo_view(&'a self) -> UTXOView<'a> {
		UTXOView::new(
			self.output_pmmr.readonly_pmmr(),
			self.header_pmmr.readonly_pmmr(),
			self.batch,
		)
	}

	/// Apply a new block to the existing state.
	///
	/// Applies the following -
	///   * header
	///   * outputs
	///   * inputs
	///   * kernels
	///
	pub fn apply_block(&mut self, b: &Block) -> Result<(), Error> {
		self.apply_header(&b.header)?;

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

		// Update the head of the extension to reflect the block we just applied.
		self.head = Tip::from_header(&b.header);

		Ok(())
	}

	fn apply_input(&mut self, input: &Input) -> Result<(), Error> {
		let commit = input.commitment();
		let pos_res = self.batch.get_output_pos(&commit);
		if let Ok(pos) = pos_res {
			// First check this input corresponds to an existing entry in the output MMR.
			if let Some(hash) = self.output_pmmr.get_hash(pos) {
				if hash != input.hash_with_index(pos - 1) {
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
						.map_err(|e| ErrorKind::TxHashSetErr(e))?;
				}
				Ok(false) => return Err(ErrorKind::AlreadySpent(commit).into()),
				Err(e) => return Err(ErrorKind::TxHashSetErr(e).into()),
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
			.push(out)
			.map_err(&ErrorKind::TxHashSetErr)?;

		// push the rangeproof to the MMR.
		let rproof_pos = self
			.rproof_pmmr
			.push(&out.proof)
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
				return Err(
					ErrorKind::Other(format!("output vs rproof MMRs different pos")).into(),
				);
			}
		}

		Ok(output_pos)
	}

	/// Push kernel onto MMR (hash and data files).
	fn apply_kernel(&mut self, kernel: &TxKernel) -> Result<(), Error> {
		self.kernel_pmmr
			.push(kernel)
			.map_err(&ErrorKind::TxHashSetErr)?;
		Ok(())
	}

	fn apply_header(&mut self, header: &BlockHeader) -> Result<(), Error> {
		self.header_pmmr
			.push(header)
			.map_err(&ErrorKind::TxHashSetErr)?;
		Ok(())
	}

	/// Get the header hash for the specified pos from the underlying MMR backend.
	fn get_header_hash(&self, pos: u64) -> Option<Hash> {
		self.header_pmmr.get_data(pos).map(|x| x.hash())
	}

	/// Get the header at the specified height based on the current state of the extension.
	/// Derives the MMR pos from the height (insertion index) and retrieves the header hash.
	/// Looks the header up in the db by hash.
	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		let pos = pmmr::insertion_to_pmmr_index(height + 1);
		if let Some(hash) = self.get_header_hash(pos) {
			let header = self.batch.get_block_header(&hash)?;
			Ok(header)
		} else {
			Err(ErrorKind::Other(format!("get header by height")).into())
		}
	}

	/// Compares the provided header to the header in the header MMR at that height.
	/// If these match we know the header is on the current chain.
	pub fn is_on_current_chain(&self, header: &BlockHeader) -> Result<(), Error> {
		let chain_header = self.get_header_by_height(header.height)?;
		if chain_header.hash() == header.hash() {
			Ok(())
		} else {
			Err(ErrorKind::Other(format!("not on current chain")).into())
		}
	}

	/// Build a Merkle proof for the given output and the block
	/// this extension is currently referencing.
	/// Note: this relies on the MMR being stable even after pruning/compaction.
	/// We need the hash of each sibling pos from the pos up to the peak
	/// including the sibling leaf node which may have been removed.
	pub fn merkle_proof(&self, output: &OutputIdentifier) -> Result<MerkleProof, Error> {
		debug!("txhashset: merkle_proof: output: {:?}", output.commit,);
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
	pub fn snapshot(&mut self) -> Result<(), Error> {
		let header = self.batch.get_block_header(&self.head.last_block_h)?;
		self.output_pmmr
			.snapshot(&header)
			.map_err(|e| ErrorKind::Other(e))?;
		self.rproof_pmmr
			.snapshot(&header)
			.map_err(|e| ErrorKind::Other(e))?;
		Ok(())
	}

	/// Rewinds the MMRs to the provided block, rewinding to the last output pos
	/// and last kernel pos of that block.
	pub fn rewind(&mut self, header: &BlockHeader) -> Result<(), Error> {
		debug!("Rewind extension to {} at {}", header.hash(), header.height,);

		// We need to build bitmaps of added and removed output positions
		// so we can correctly rewind all operations applied to the output MMR
		// after the position we are rewinding to (these operations will be
		// undone during rewind).
		// Rewound output pos will be removed from the MMR.
		// Rewound input (spent) pos will be added back to the MMR.
		let head_header = self.batch.get_block_header(&self.head.last_block_h)?;
		let rewind_rm_pos = input_pos_to_rewind(header, &head_header, &self.batch)?;

		let header_pos = pmmr::insertion_to_pmmr_index(header.height + 1);

		self.rewind_to_pos(
			header_pos,
			header.output_mmr_size,
			header.kernel_mmr_size,
			&rewind_rm_pos,
		)?;

		// Update our head to reflect the header we rewound to.
		self.head = Tip::from_header(header);

		Ok(())
	}

	/// Rewinds the MMRs to the provided positions, given the output and
	/// kernel we want to rewind to.
	fn rewind_to_pos(
		&mut self,
		header_pos: u64,
		output_pos: u64,
		kernel_pos: u64,
		rewind_rm_pos: &Bitmap,
	) -> Result<(), Error> {
		debug!(
			"txhashset: rewind_to_pos: header {}, output {}, kernel {}",
			header_pos, output_pos, kernel_pos,
		);

		self.header_pmmr
			.rewind(header_pos, &Bitmap::create())
			.map_err(&ErrorKind::TxHashSetErr)?;
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
			header_root: self.header_pmmr.root(),
			output_root: self.output_pmmr.root(),
			rproof_root: self.rproof_pmmr.root(),
			kernel_root: self.kernel_pmmr.root(),
		}
	}

	/// Get the root of the current header MMR.
	pub fn header_root(&self) -> Hash {
		self.header_pmmr.root()
	}

	/// Validate the following MMR roots against the latest header applied -
	///   * output
	///   * rangeproof
	///   * kernel
	///
	/// Note we do not validate the header MMR root here as we need to validate
	/// a header against the state of the MMR *prior* to applying it.
	/// Each header commits to the root of the MMR of all previous headers,
	/// not including the header itself.
	///
	pub fn validate_roots(&self) -> Result<(), Error> {
		// If we are validating the genesis block then we have no outputs or
		// kernels. So we are done here.
		if self.head.height == 0 {
			return Ok(());
		}
		let head_header = self.batch.get_block_header(&self.head.last_block_h)?;
		let roots = self.roots();
		if roots.output_root != head_header.output_root
			|| roots.rproof_root != head_header.range_proof_root
			|| roots.kernel_root != head_header.kernel_root
		{
			Err(ErrorKind::InvalidRoot.into())
		} else {
			Ok(())
		}
	}

	/// Validate the provided header by comparing its prev_root to the
	/// root of the current header MMR.
	pub fn validate_header_root(&self, header: &BlockHeader) -> Result<(), Error> {
		if header.height == 0 {
			return Ok(());
		}
		let roots = self.roots();
		if roots.header_root != header.prev_root {
			Err(ErrorKind::InvalidRoot.into())
		} else {
			Ok(())
		}
	}

	/// Validate the header, output and kernel MMR sizes against the block header.
	pub fn validate_sizes(&self) -> Result<(), Error> {
		// If we are validating the genesis block then we have no outputs or
		// kernels. So we are done here.
		if self.head.height == 0 {
			return Ok(());
		}

		let head_header = self.batch.get_block_header(&self.head.last_block_h)?;
		let (header_mmr_size, output_mmr_size, rproof_mmr_size, kernel_mmr_size) = self.sizes();
		let expected_header_mmr_size =
			pmmr::insertion_to_pmmr_index(self.head.height + 2).saturating_sub(1);

		if header_mmr_size != expected_header_mmr_size {
			Err(ErrorKind::InvalidMMRSize.into())
		} else if output_mmr_size != head_header.output_mmr_size {
			Err(ErrorKind::InvalidMMRSize.into())
		} else if kernel_mmr_size != head_header.kernel_mmr_size {
			Err(ErrorKind::InvalidMMRSize.into())
		} else if output_mmr_size != rproof_mmr_size {
			Err(ErrorKind::InvalidMMRSize.into())
		} else {
			Ok(())
		}
	}

	fn validate_mmrs(&self) -> Result<(), Error> {
		let now = Instant::now();

		// validate all hashes and sums within the trees
		if let Err(e) = self.header_pmmr.validate() {
			return Err(ErrorKind::InvalidTxHashSet(e).into());
		}
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
			"txhashset: validated the header {}, output {}, rproof {}, kernel {} mmrs, took {}s",
			self.header_pmmr.unpruned_size(),
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
	pub fn validate_kernel_sums(&self) -> Result<((Commitment, Commitment)), Error> {
		let now = Instant::now();

		let head_header = self.batch.get_block_header(&self.head.last_block_h)?;
		let genesis = self.get_header_by_height(0)?;
		let (utxo_sum, kernel_sum) = self.verify_kernel_sums(
			head_header.total_overage(genesis.kernel_mmr_size > 0),
			head_header.total_kernel_offset(),
		)?;

		debug!(
			"txhashset: validated total kernel sums, took {}s",
			now.elapsed().as_secs(),
		);

		Ok((utxo_sum, kernel_sum))
	}

	/// Validate the txhashset state against the provided block header.
	/// A "fast validation" will skip rangeproof verification and kernel signature verification.
	pub fn validate(
		&self,
		fast_validation: bool,
		status: &dyn TxHashsetWriteStatus,
	) -> Result<((Commitment, Commitment)), Error> {
		self.validate_mmrs()?;
		self.validate_roots()?;
		self.validate_sizes()?;

		if self.head.height == 0 {
			let zero_commit = secp_static::commit_to_zero_value();
			return Ok((zero_commit.clone(), zero_commit.clone()));
		}

		// The real magicking happens here. Sum of kernel excesses should equal
		// sum of unspent outputs minus total supply.
		let (output_sum, kernel_sum) = self.validate_kernel_sums()?;

		// These are expensive verification step (skipped for "fast validation").
		if !fast_validation {
			// Verify the rangeproof associated with each unspent output.
			self.verify_rangeproofs(status)?;

			// Verify all the kernel signatures.
			self.verify_kernel_signatures(status)?;
		}

		Ok((output_sum, kernel_sum))
	}

	/// Rebuild the index of MMR positions to the corresponding UTXOs.
	/// This is a costly operation performed only when we receive a full new chain state.
	pub fn rebuild_index(&self) -> Result<(), Error> {
		let now = Instant::now();

		self.batch.clear_output_pos()?;

		let mut count = 0;
		for pos in self.output_pmmr.leaf_pos_iter() {
			if let Some(out) = self.output_pmmr.get_data(pos) {
				self.batch.save_output_pos(&out.commit, pos)?;
				count += 1;
			}
		}

		debug!(
			"txhashset: rebuild_index: {} UTXOs, took {}s",
			count,
			now.elapsed().as_secs(),
		);

		Ok(())
	}

	/// Force the rollback of this extension, no matter the result
	pub fn force_rollback(&mut self) {
		self.rollback = true;
	}

	/// Dumps the output MMR.
	/// We use this after compacting for visual confirmation that it worked.
	pub fn dump_output_pmmr(&self) {
		debug!("-- outputs --");
		self.output_pmmr.dump_from_file(false);
		debug!("--");
		self.output_pmmr.dump_stats();
		debug!("-- end of outputs --");
	}

	/// Dumps the state of the 3 sum trees to stdout for debugging. Short
	/// version only prints the Output tree.
	pub fn dump(&self, short: bool) {
		debug!("-- outputs --");
		self.output_pmmr.dump(short);
		if !short {
			debug!("-- range proofs --");
			self.rproof_pmmr.dump(short);
			debug!("-- kernels --");
			self.kernel_pmmr.dump(short);
		}
	}

	/// Sizes of each of the sum trees
	pub fn sizes(&self) -> (u64, u64, u64, u64) {
		(
			self.header_pmmr.unpruned_size(),
			self.output_pmmr.unpruned_size(),
			self.rproof_pmmr.unpruned_size(),
			self.kernel_pmmr.unpruned_size(),
		)
	}

	fn verify_kernel_signatures(&self, status: &dyn TxHashsetWriteStatus) -> Result<(), Error> {
		let now = Instant::now();

		let mut kern_count = 0;
		let total_kernels = pmmr::n_leaves(self.kernel_pmmr.unpruned_size());
		for n in 1..self.kernel_pmmr.unpruned_size() + 1 {
			if pmmr::is_leaf(n) {
				let kernel = self
					.kernel_pmmr
					.get_data(n)
					.ok_or::<Error>(ErrorKind::TxKernelNotFound.into())?;

				kernel.verify()?;
				kern_count += 1;

				if kern_count % 20 == 0 {
					status.on_validation(kern_count, total_kernels, 0, 0);
				}
				if kern_count % 1_000 == 0 {
					debug!(
						"txhashset: verify_kernel_signatures: verified {} signatures",
						kern_count,
					);
				}
			}
		}

		debug!(
			"txhashset: verified {} kernel signatures, pmmr size {}, took {}s",
			kern_count,
			self.kernel_pmmr.unpruned_size(),
			now.elapsed().as_secs(),
		);

		Ok(())
	}

	fn verify_rangeproofs(&self, status: &dyn TxHashsetWriteStatus) -> Result<(), Error> {
		let now = Instant::now();

		let mut commits: Vec<Commitment> = vec![];
		let mut proofs: Vec<RangeProof> = vec![];

		let mut proof_count = 0;
		let total_rproofs = pmmr::n_leaves(self.output_pmmr.unpruned_size());
		for pos in self.output_pmmr.leaf_pos_iter() {
			let output = self.output_pmmr.get_data(pos);
			let proof = self.rproof_pmmr.get_data(pos);

			// Output and corresponding rangeproof *must* exist.
			// It is invalid for either to be missing and we fail immediately in this case.
			match (output, proof) {
				(None, _) => return Err(ErrorKind::OutputNotFound.into()),
				(_, None) => return Err(ErrorKind::RangeproofNotFound.into()),
				(Some(output), Some(proof)) => {
					commits.push(output.commit);
					proofs.push(proof);
				}
			}

			proof_count += 1;

			if proofs.len() >= 1_000 {
				Output::batch_verify_proofs(&commits, &proofs)?;
				commits.clear();
				proofs.clear();
				debug!(
					"txhashset: verify_rangeproofs: verified {} rangeproofs",
					proof_count,
				);
			}

			if proof_count % 20 == 0 {
				status.on_validation(0, 0, proof_count, total_rproofs);
			}
		}

		// remaining part which not full of 1000 range proofs
		if proofs.len() > 0 {
			Output::batch_verify_proofs(&commits, &proofs)?;
			commits.clear();
			proofs.clear();
			debug!(
				"txhashset: verify_rangeproofs: verified {} rangeproofs",
				proof_count,
			);
		}

		debug!(
			"txhashset: verified {} rangeproofs, pmmr size {}, took {}s",
			proof_count,
			self.rproof_pmmr.unpruned_size(),
			now.elapsed().as_secs(),
		);
		Ok(())
	}
}

/// Packages the txhashset data files into a zip and returns a Read to the
/// resulting file
pub fn zip_read(root_dir: String, header: &BlockHeader) -> Result<File, Error> {
	let txhashset_zip = format!("{}_{}.zip", TXHASHSET_ZIP, header.hash().to_string());

	let txhashset_path = Path::new(&root_dir).join(TXHASHSET_SUBDIR);
	let zip_path = Path::new(&root_dir).join(txhashset_zip);

	// if file exist, just re-use it
	let zip_file = File::open(zip_path.clone());
	if let Ok(zip) = zip_file {
		return Ok(zip);
	} else {
		// clean up old zips.
		// Theoretically, we only need clean-up those zip files older than STATE_SYNC_THRESHOLD.
		// But practically, these zip files are not small ones, we just keep the zips in last 24 hours
		let data_dir = Path::new(&root_dir);
		let pattern = format!("{}_", TXHASHSET_ZIP);
		if let Ok(n) = clean_files_by_prefix(data_dir.clone(), &pattern, 24 * 60 * 60) {
			debug!(
				"{} zip files have been clean up in folder: {:?}",
				n, data_dir
			);
		}
	}

	// otherwise, create the zip archive
	let path_to_be_cleanup = {
		// Temp txhashset directory
		let temp_txhashset_path = Path::new(&root_dir).join(format!(
			"{}_zip_{}",
			TXHASHSET_SUBDIR,
			header.hash().to_string()
		));
		// Remove temp dir if it exist
		if temp_txhashset_path.exists() {
			fs::remove_dir_all(&temp_txhashset_path)?;
		}
		// Copy file to another dir
		file::copy_dir_to(&txhashset_path, &temp_txhashset_path)?;

		let zip_file = File::create(zip_path.clone())?;

		// Explicit list of files to add to our zip archive.
		let files = file_list(header);

		zip::create_zip(&zip_file, &temp_txhashset_path, files)?;

		temp_txhashset_path
	};

	// open it again to read it back
	let zip_file = File::open(zip_path.clone())?;

	// clean-up temp txhashset directory.
	if let Err(e) = fs::remove_dir_all(&path_to_be_cleanup) {
		warn!(
			"txhashset zip file: {:?} fail to remove, err: {}",
			zip_path.to_str(),
			e
		);
	}
	Ok(zip_file)
}

// Explicit list of files to extract from our zip archive.
// We include *only* these files when building the txhashset zip.
// We extract *only* these files when receiving a txhashset zip.
// Everything else will be safely ignored.
// Return Vec<PathBuf> as some of these are dynamic (specifically the "rewound" leaf files).
fn file_list(header: &BlockHeader) -> Vec<PathBuf> {
	vec![
		// kernel MMR
		PathBuf::from("kernel/pmmr_data.bin"),
		PathBuf::from("kernel/pmmr_hash.bin"),
		// output MMR
		PathBuf::from("output/pmmr_data.bin"),
		PathBuf::from("output/pmmr_hash.bin"),
		PathBuf::from("output/pmmr_prun.bin"),
		// rangeproof MMR
		PathBuf::from("rangeproof/pmmr_data.bin"),
		PathBuf::from("rangeproof/pmmr_hash.bin"),
		PathBuf::from("rangeproof/pmmr_prun.bin"),
		// Header specific "rewound" leaf files for output and rangeproof MMR.
		PathBuf::from(format!("output/pmmr_leaf.bin.{}", header.hash())),
		PathBuf::from(format!("rangeproof/pmmr_leaf.bin.{}", header.hash())),
	]
}

/// Extract the txhashset data from a zip file and writes the content into the
/// txhashset storage dir
pub fn zip_write(
	root_dir: PathBuf,
	txhashset_data: File,
	header: &BlockHeader,
) -> Result<(), Error> {
	debug!("zip_write on path: {:?}", root_dir);
	let txhashset_path = root_dir.clone().join(TXHASHSET_SUBDIR);
	fs::create_dir_all(&txhashset_path)?;

	// Explicit list of files to extract from our zip archive.
	let files = file_list(header);

	// We expect to see *exactly* the paths listed above.
	// No attempt is made to be permissive or forgiving with "alternative" paths.
	// These are the *only* files we will attempt to extract from the zip file.
	// If any of these are missing we will attempt to continue as some are potentially optional.
	zip::extract_files(txhashset_data, &txhashset_path, files)?;
	Ok(())
}

/// Overwrite txhashset folders in "to" folder with "from" folder
pub fn txhashset_replace(from: PathBuf, to: PathBuf) -> Result<(), Error> {
	debug!("txhashset_replace: move from {:?} to {:?}", from, to);

	// clean the 'to' folder firstly
	clean_txhashset_folder(&to);

	// rename the 'from' folder as the 'to' folder
	if let Err(e) = fs::rename(
		from.clone().join(TXHASHSET_SUBDIR),
		to.clone().join(TXHASHSET_SUBDIR),
	) {
		error!("hashset_replace fail on {}. err: {}", TXHASHSET_SUBDIR, e);
		Err(ErrorKind::TxHashSetErr(format!("txhashset replacing fail")).into())
	} else {
		Ok(())
	}
}

/// Clean the txhashset folder
pub fn clean_txhashset_folder(root_dir: &PathBuf) {
	let txhashset_path = root_dir.clone().join(TXHASHSET_SUBDIR);
	if txhashset_path.exists() {
		if let Err(e) = fs::remove_dir_all(txhashset_path.clone()) {
			warn!(
				"clean_txhashset_folder: fail on {:?}. err: {}",
				txhashset_path, e
			);
		}
	}
}

/// Clean the header folder
pub fn clean_header_folder(root_dir: &PathBuf) {
	let header_path = root_dir.clone().join(HEADERHASHSET_SUBDIR);
	if header_path.exists() {
		if let Err(e) = fs::remove_dir_all(header_path.clone()) {
			warn!("clean_header_folder: fail on {:?}. err: {}", header_path, e);
		}
	}
}

/// Given a block header to rewind to and the block header at the
/// head of the current chain state, we need to calculate the positions
/// of all inputs (spent outputs) we need to "undo" during a rewind.
/// We do this by leveraging the "block_input_bitmap" cache and OR'ing
/// the set of bitmaps together for the set of blocks being rewound.
pub fn input_pos_to_rewind(
	block_header: &BlockHeader,
	head_header: &BlockHeader,
	batch: &Batch<'_>,
) -> Result<Bitmap, Error> {
	if head_header.height < block_header.height {
		debug!(
			"input_pos_to_rewind: {} < {}, nothing to rewind",
			head_header.height, block_header.height
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

	let mut current = head_header.clone();
	while current.hash() != block_header.hash() {
		if current.height < 1 {
			break;
		}

		// I/O should be minimized or eliminated here for most
		// rewind scenarios.
		if let Ok(b_res) = batch.get_block_input_bitmap(&current.hash()) {
			bitmap_fast_or(Some(b_res), &mut block_input_bitmaps);
		}
		current = batch.get_previous_header(&current)?;
	}

	bitmap_fast_or(None, &mut block_input_bitmaps).ok_or_else(|| ErrorKind::Bitmap.into())
}
