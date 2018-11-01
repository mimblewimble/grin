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
use core::core::pmmr::{self, ReadonlyPMMR, RewindablePMMR, DBPMMR, PMMR};
use core::core::{Block, BlockHeader, Input, Output, OutputFeatures, OutputIdentifier, TxKernel};
use core::global;
use core::ser::{PMMRIndexHashable, PMMRable};

use error::{Error, ErrorKind};
use grin_store;
use grin_store::pmmr::{HashOnlyMMRBackend, PMMRBackend, PMMR_FILES};
use grin_store::types::prune_noop;
use store::{Batch, ChainStore};
use txhashset::{RewindableKernelView, UTXOView};
use types::{Tip, TxHashSetRoots, TxHashsetWriteStatus};
use util::{file, secp_static, zip};

const HEADERHASHSET_SUBDIR: &'static str = "header";
const TXHASHSET_SUBDIR: &'static str = "txhashset";

const HEADER_HEAD_SUBDIR: &'static str = "header_head";
const SYNC_HEAD_SUBDIR: &'static str = "sync_head";

const OUTPUT_SUBDIR: &'static str = "output";
const RANGE_PROOF_SUBDIR: &'static str = "rangeproof";
const KERNEL_SUBDIR: &'static str = "kernel";

const TXHASHSET_ZIP: &'static str = "txhashset_snapshot.zip";

struct HashOnlyMMRHandle {
	backend: HashOnlyMMRBackend,
	last_pos: u64,
}

impl HashOnlyMMRHandle {
	fn new(root_dir: &str, sub_dir: &str, file_name: &str) -> Result<HashOnlyMMRHandle, Error> {
		let path = Path::new(root_dir).join(sub_dir).join(file_name);
		fs::create_dir_all(path.clone())?;
		let backend = HashOnlyMMRBackend::new(path.to_str().unwrap())?;
		let last_pos = backend.unpruned_size()?;
		Ok(HashOnlyMMRHandle { backend, last_pos })
	}
}

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
		root_dir: &str,
		sub_dir: &str,
		file_name: &str,
		prunable: bool,
		header: Option<&BlockHeader>,
	) -> Result<PMMRHandle<T>, Error> {
		let path = Path::new(root_dir).join(sub_dir).join(file_name);
		fs::create_dir_all(path.clone())?;
		let backend = PMMRBackend::new(path.to_str().unwrap().to_string(), prunable, header)?;
		let last_pos = backend.unpruned_size()?;
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
	/// Note: the header MMR is backed by the database maintains just the hash file.
	header_pmmr_h: HashOnlyMMRHandle,

	/// Header MMR to support exploratory sync_head.
	/// The header_head and sync_head chains can diverge so we need to maintain
	/// multiple header MMRs during the sync process.
	///
	/// Note: this is rewound and applied separately to the other MMRs
	/// via a "sync_extension".
	/// Note: the sync MMR is backed by the database and maintains just the hash file.
	sync_pmmr_h: HashOnlyMMRHandle,

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
		Ok(TxHashSet {
			header_pmmr_h: HashOnlyMMRHandle::new(
				&root_dir,
				HEADERHASHSET_SUBDIR,
				HEADER_HEAD_SUBDIR,
			)?,
			sync_pmmr_h: HashOnlyMMRHandle::new(&root_dir, HEADERHASHSET_SUBDIR, SYNC_HEAD_SUBDIR)?,
			output_pmmr_h: PMMRHandle::new(
				&root_dir,
				TXHASHSET_SUBDIR,
				OUTPUT_SUBDIR,
				true,
				header,
			)?,
			rproof_pmmr_h: PMMRHandle::new(
				&root_dir,
				TXHASHSET_SUBDIR,
				RANGE_PROOF_SUBDIR,
				true,
				header,
			)?,
			kernel_pmmr_h: PMMRHandle::new(
				&root_dir,
				TXHASHSET_SUBDIR,
				KERNEL_SUBDIR,
				false,
				None,
			)?,
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

	/// Get MMR roots.
	pub fn roots(&mut self) -> TxHashSetRoots {
		let header_pmmr: DBPMMR<BlockHeader, _> =
			DBPMMR::at(&mut self.header_pmmr_h.backend, self.header_pmmr_h.last_pos);
		let output_pmmr: PMMR<OutputIdentifier, _> =
			PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
		let rproof_pmmr: PMMR<RangeProof, _> =
			PMMR::at(&mut self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos);
		let kernel_pmmr: PMMR<TxKernel, _> =
			PMMR::at(&mut self.kernel_pmmr_h.backend, self.kernel_pmmr_h.last_pos);

		TxHashSetRoots {
			header_root: header_pmmr.root(),
			output_root: output_pmmr.root(),
			rproof_root: rproof_pmmr.root(),
			kernel_root: kernel_pmmr.root(),
		}
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

		let rewind_rm_pos = input_pos_to_rewind(&horizon_header, &head_header, &batch)?;

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
	let commit_index = trees.commit_index.clone();
	let batch = commit_index.batch()?;

	// We want to use the current head of the most work chain unless
	// we explicitly rewind the extension.
	let header = batch.head_header()?;

	trace!("Starting new txhashset (readonly) extension.");

	let res = {
		let mut extension = Extension::new(trees, &batch, header);
		extension.force_rollback();

		// TODO - header_mmr may be out ahead via the header_head
		// TODO - do we need to handle this via an explicit rewind on the header_mmr?

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
pub fn utxo_view<'a, F, T>(trees: &'a TxHashSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&UTXOView) -> Result<T, Error>,
{
	let res: Result<T, Error>;
	{
		let output_pmmr =
			ReadonlyPMMR::at(&trees.output_pmmr_h.backend, trees.output_pmmr_h.last_pos);

		// Create a new batch here to pass into the utxo_view.
		// Discard it (rollback) after we finish with the utxo_view.
		let batch = trees.commit_index.batch()?;
		let utxo = UTXOView::new(output_pmmr, &batch);
		res = inner(&utxo);
	}
	res
}

/// Rewindable (but still readonly) view on the kernel MMR.
/// The underlying backend is readonly. But we permit the PMMR to be "rewound"
/// via last_pos.
/// We create a new db batch for this view and discard it (rollback)
/// when we are done with the view.
pub fn rewindable_kernel_view<'a, F, T>(trees: &'a TxHashSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&mut RewindableKernelView) -> Result<T, Error>,
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
	batch: &'a mut Batch,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut Extension) -> Result<T, Error>,
{
	let sizes: (u64, u64, u64, u64);
	let res: Result<T, Error>;
	let rollback: bool;

	// We want to use the current head of the most work chain unless
	// we explicitly rewind the extension.
	let header = batch.head_header()?;

	// create a child transaction so if the state is rolled back by itself, all
	// index saving can be undone
	let child_batch = batch.child()?;
	{
		trace!("Starting new txhashset extension.");

		// TODO - header_mmr may be out ahead via the header_head
		// TODO - do we need to handle this via an explicit rewind on the header_mmr?
		let mut extension = Extension::new(trees, &child_batch, header);
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
				trees.header_pmmr_h.backend.sync()?;
				trees.output_pmmr_h.backend.sync()?;
				trees.rproof_pmmr_h.backend.sync()?;
				trees.kernel_pmmr_h.backend.sync()?;
				trees.header_pmmr_h.last_pos = sizes.0;
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
	batch: &'a mut Batch,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut HeaderExtension) -> Result<T, Error>,
{
	let size: u64;
	let res: Result<T, Error>;
	let rollback: bool;

	// We want to use the current sync_head unless
	// we explicitly rewind the extension.
	let head = batch.get_sync_head()?;
	let header = batch.get_block_header(&head.last_block_h)?;

	// create a child transaction so if the state is rolled back by itself, all
	// index saving can be undone
	let child_batch = batch.child()?;
	{
		trace!("Starting new txhashset sync_head extension.");
		let pmmr = DBPMMR::at(&mut trees.sync_pmmr_h.backend, trees.sync_pmmr_h.last_pos);
		let mut extension = HeaderExtension::new(pmmr, &child_batch, header);

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
	batch: &'a mut Batch,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut HeaderExtension) -> Result<T, Error>,
{
	let size: u64;
	let res: Result<T, Error>;
	let rollback: bool;

	// We want to use the current head of the most work chain unless
	// we explicitly rewind the extension.
	let head = batch.head()?;
	let header = batch.get_block_header(&head.last_block_h)?;

	// create a child transaction so if the state is rolled back by itself, all
	// index saving can be undone
	let child_batch = batch.child()?;
	{
		trace!("Starting new txhashset header extension.");
		let pmmr = DBPMMR::at(
			&mut trees.header_pmmr_h.backend,
			trees.header_pmmr_h.last_pos,
		);
		let mut extension = HeaderExtension::new(pmmr, &child_batch, header);
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
	header: BlockHeader,

	pmmr: DBPMMR<'a, BlockHeader, HashOnlyMMRBackend>,

	/// Rollback flag.
	rollback: bool,

	/// Batch in which the extension occurs, public so it can be used within
	/// an `extending` closure. Just be careful using it that way as it will
	/// get rolled back with the extension (i.e on a losing fork).
	pub batch: &'a Batch<'a>,
}

impl<'a> HeaderExtension<'a> {
	fn new(
		pmmr: DBPMMR<'a, BlockHeader, HashOnlyMMRBackend>,
		batch: &'a Batch,
		header: BlockHeader,
	) -> HeaderExtension<'a> {
		HeaderExtension {
			header,
			pmmr,
			rollback: false,
			batch,
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
		self.header = header.clone();
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
			.rewind(header_pos)
			.map_err(&ErrorKind::TxHashSetErr)?;

		// Update our header to reflect the one we rewound to.
		self.header = header.clone();

		Ok(())
	}

	/// Truncate the header MMR (rewind all the way back to pos 0).
	/// Used when rebuilding the header MMR by reapplying all headers
	/// including the genesis block header.
	pub fn truncate(&mut self) -> Result<(), Error> {
		debug!("Truncating header extension.");
		self.pmmr.rewind(0).map_err(&ErrorKind::TxHashSetErr)?;
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
	header: BlockHeader,

	header_pmmr: DBPMMR<'a, BlockHeader, HashOnlyMMRBackend>,
	output_pmmr: PMMR<'a, OutputIdentifier, PMMRBackend<OutputIdentifier>>,
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
	fn new(trees: &'a mut TxHashSet, batch: &'a Batch, header: BlockHeader) -> Extension<'a> {
		Extension {
			header,
			header_pmmr: DBPMMR::at(
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

	/// Build a view of the current UTXO set based on the output PMMR.
	pub fn utxo_view(&'a self) -> UTXOView<'a> {
		UTXOView::new(self.output_pmmr.readonly_pmmr(), self.batch)
	}

	// TODO - move this into "utxo_view"
	/// Verify we are not attempting to spend any coinbase outputs
	/// that have not sufficiently matured.
	pub fn verify_coinbase_maturity(&self, inputs: &Vec<Input>, height: u64) -> Result<(), Error> {
		// Find the greatest output pos of any coinbase
		// outputs we are attempting to spend.
		let pos = inputs
			.iter()
			.filter(|x| x.features.contains(OutputFeatures::COINBASE_OUTPUT))
			.filter_map(|x| self.batch.get_output_pos(&x.commitment()).ok())
			.max()
			.unwrap_or(0);

		if pos > 0 {
			// If we have not yet reached 1,000 / 1,440 blocks then
			// we can fail immediately as coinbase cannot be mature.
			if height < global::coinbase_maturity() {
				return Err(ErrorKind::ImmatureCoinbase.into());
			}

			// Find the "cutoff" pos in the output MMR based on the
			// header from 1,000 blocks ago.
			let cutoff_height = height.checked_sub(global::coinbase_maturity()).unwrap_or(0);
			let cutoff_header = self.batch.get_header_by_height(cutoff_height)?;
			let cutoff_pos = cutoff_header.output_mmr_size;

			// If any output pos exceed the cutoff_pos
			// we know they have not yet sufficiently matured.
			if pos > cutoff_pos {
				return Err(ErrorKind::ImmatureCoinbase.into());
			}
		}

		Ok(())
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

		// Update the header on the extension to reflect the block we just applied.
		self.header = b.header.clone();

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

	/// Push kernel onto MMR (hash and data files).
	fn apply_kernel(&mut self, kernel: &TxKernel) -> Result<(), Error> {
		self.kernel_pmmr
			.push(kernel.clone())
			.map_err(&ErrorKind::TxHashSetErr)?;
		Ok(())
	}

	fn apply_header(&mut self, header: &BlockHeader) -> Result<(), Error> {
		self.header_pmmr
			.push(&header)
			.map_err(&ErrorKind::TxHashSetErr)?;
		Ok(())
	}

	/// TODO - move this into "utxo_view"
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
		self.output_pmmr
			.snapshot(&self.header)
			.map_err(|e| ErrorKind::Other(e))?;
		self.rproof_pmmr
			.snapshot(&self.header)
			.map_err(|e| ErrorKind::Other(e))?;
		Ok(())
	}

	/// Rewinds the MMRs to the provided block, rewinding to the last output pos
	/// and last kernel pos of that block.
	pub fn rewind(&mut self, header: &BlockHeader) -> Result<(), Error> {
		debug!("Rewind to header {} at {}", header.hash(), header.height,);

		// We need to build bitmaps of added and removed output positions
		// so we can correctly rewind all operations applied to the output MMR
		// after the position we are rewinding to (these operations will be
		// undone during rewind).
		// Rewound output pos will be removed from the MMR.
		// Rewound input (spent) pos will be added back to the MMR.
		let rewind_rm_pos = input_pos_to_rewind(header, &self.header, &self.batch)?;

		let header_pos = pmmr::insertion_to_pmmr_index(header.height + 1);

		self.rewind_to_pos(
			header_pos,
			header.output_mmr_size,
			header.kernel_mmr_size,
			&rewind_rm_pos,
		)?;

		// Update our header to reflect the one we rewound to.
		self.header = header.clone();

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
			.rewind(header_pos)
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
		if self.header.height == 0 {
			return Ok(());
		}

		let roots = self.roots();

		if roots.output_root != self.header.output_root
			|| roots.rproof_root != self.header.range_proof_root
			|| roots.kernel_root != self.header.kernel_root
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
		if self.header.height == 0 {
			return Ok(());
		}

		let (header_mmr_size, output_mmr_size, rproof_mmr_size, kernel_mmr_size) = self.sizes();
		let expected_header_mmr_size = pmmr::insertion_to_pmmr_index(self.header.height + 2) - 1;

		if header_mmr_size != expected_header_mmr_size {
			Err(ErrorKind::InvalidMMRSize.into())
		} else if output_mmr_size != self.header.output_mmr_size {
			Err(ErrorKind::InvalidMMRSize.into())
		} else if kernel_mmr_size != self.header.kernel_mmr_size {
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
		let (utxo_sum, kernel_sum) = self.verify_kernel_sums(
			self.header.total_overage(),
			self.header.total_kernel_offset(),
		)?;
		Ok((utxo_sum, kernel_sum))
	}

	/// Validate the txhashset state against the provided block header.
	/// A "fast validation" will skip rangeproof verification and kernel signature verification.
	pub fn validate(
		&self,
		fast_validation: bool,
		status: &TxHashsetWriteStatus,
	) -> Result<((Commitment, Commitment)), Error> {
		self.validate_mmrs()?;
		self.validate_roots()?;
		self.validate_sizes()?;

		if self.header.height == 0 {
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
							"txhashset: verify_rangeproofs: verified {} rangeproofs",
							proof_count,
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
		debug!("Unexpected folder(s) found in txhashset folder, removing.");
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
pub fn input_pos_to_rewind(
	block_header: &BlockHeader,
	head_header: &BlockHeader,
	batch: &Batch,
) -> Result<Bitmap, Error> {
	let mut current = head_header.hash();
	let mut height = head_header.height;

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
		current = batch.get_hash_by_height(height)?;
	}

	let bitmap = bitmap_fast_or(None, &mut block_input_bitmaps).unwrap();
	Ok(bitmap)
}
