// Copyright 2021 The Grin Developers
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

use crate::core::consensus::WEEK_HEIGHT;
use crate::core::core::committed::Committed;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::merkle_proof::MerkleProof;
use crate::core::core::pmmr::{
	self, Backend, ReadablePMMR, ReadonlyPMMR, RewindablePMMR, VecBackend, PMMR,
};
use crate::core::core::{
	Block, BlockHeader, KernelFeatures, Output, OutputIdentifier, Segment, TxKernel,
};
use crate::core::global;
use crate::core::ser::{PMMRable, ProtocolVersion};
use crate::error::Error;
use crate::linked_list::{ListIndex, PruneableListIndex, RewindableListIndex};
use crate::store::{self, Batch, ChainStore};
use crate::txhashset::bitmap_accumulator::{BitmapAccumulator, BitmapChunk};
use crate::txhashset::{RewindableKernelView, UTXOView};
use crate::types::{CommitPos, OutputRoots, Tip, TxHashSetRoots, TxHashsetWriteStatus};
use crate::util::secp::pedersen::{Commitment, RangeProof};
use crate::util::{file, secp_static, zip, StopState};
use crate::SyncState;
use croaring::Bitmap;
use grin_store::pmmr::{clean_files_by_prefix, PMMRBackend};
use std::cmp::Ordering;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const TXHASHSET_SUBDIR: &str = "txhashset";

const OUTPUT_SUBDIR: &str = "output";
const RANGE_PROOF_SUBDIR: &str = "rangeproof";
const KERNEL_SUBDIR: &str = "kernel";

const TXHASHSET_ZIP: &str = "txhashset_snapshot";

/// Convenience enum to keep track of hash and leaf insertions when rebuilding an mmr
/// from segments
#[derive(Eq)]
enum OrderedHashLeafNode {
	/// index of data in hashes array, pmmr position
	Hash(usize, u64),
	/// index of data in leaf_data array, pmmr position
	Leaf(usize, u64),
}

impl PartialEq for OrderedHashLeafNode {
	fn eq(&self, other: &Self) -> bool {
		let a_val = match self {
			OrderedHashLeafNode::Hash(_, pos0) => pos0,
			OrderedHashLeafNode::Leaf(_, pos0) => pos0,
		};
		let b_val = match other {
			OrderedHashLeafNode::Hash(_, pos0) => pos0,
			OrderedHashLeafNode::Leaf(_, pos0) => pos0,
		};
		a_val == b_val
	}
}

impl Ord for OrderedHashLeafNode {
	fn cmp(&self, other: &Self) -> Ordering {
		let a_val = match self {
			OrderedHashLeafNode::Hash(_, pos0) => pos0,
			OrderedHashLeafNode::Leaf(_, pos0) => pos0,
		};
		let b_val = match other {
			OrderedHashLeafNode::Hash(_, pos0) => pos0,
			OrderedHashLeafNode::Leaf(_, pos0) => pos0,
		};
		a_val.cmp(&b_val)
	}
}

impl PartialOrd for OrderedHashLeafNode {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		let a_val = match self {
			OrderedHashLeafNode::Hash(_, pos0) => pos0,
			OrderedHashLeafNode::Leaf(_, pos0) => pos0,
		};
		let b_val = match other {
			OrderedHashLeafNode::Hash(_, pos0) => pos0,
			OrderedHashLeafNode::Leaf(_, pos0) => pos0,
		};
		Some(a_val.cmp(b_val))
	}
}

/// Convenience wrapper around a single prunable MMR backend.
pub struct PMMRHandle<T: PMMRable> {
	/// The backend storage for the MMR.
	pub backend: PMMRBackend<T>,
	/// The MMR size accessible via this handle (backend may continue out beyond this).
	pub size: u64,
}

impl<T: PMMRable> PMMRHandle<T> {
	/// Constructor to create a PMMR handle from an existing directory structure on disk.
	/// Creates the backend files as necessary if they do not already exist.
	pub fn new<P: AsRef<Path>>(
		path: P,
		prunable: bool,
		version: ProtocolVersion,
		header: Option<&BlockHeader>,
	) -> Result<PMMRHandle<T>, Error> {
		fs::create_dir_all(&path)?;
		let backend = PMMRBackend::new(&path, prunable, version, header)?;
		let size = backend.unpruned_size();
		Ok(PMMRHandle { backend, size })
	}
}

impl PMMRHandle<BlockHeader> {
	/// Used during chain init to ensure the header PMMR is consistent with header_head in the db.
	pub fn init_head(&mut self, head: &Tip) -> Result<(), Error> {
		let head_hash = self.head_hash()?;
		let expected_hash = self.get_header_hash_by_height(head.height)?;
		if head.hash() != expected_hash {
			error!(
				"header PMMR inconsistent: {} vs {} at {}",
				expected_hash,
				head.hash(),
				head.height
			);
			return Err(Error::Other("header PMMR inconsistent".to_string()));
		}

		// use next header pos to find our size.
		let next_height = head.height + 1;
		let size = pmmr::insertion_to_pmmr_index(next_height);

		debug!(
			"init_head: header PMMR: current head {} at pos {}",
			head_hash, self.size
		);
		debug!(
			"init_head: header PMMR: resetting to {} at pos {} (height {})",
			head.hash(),
			size,
			head.height
		);

		self.size = size;
		Ok(())
	}

	/// Get the header hash at the specified height based on the current header MMR state.
	pub fn get_header_hash_by_height(&self, height: u64) -> Result<Hash, Error> {
		if height >= self.size {
			return Err(Error::InvalidHeaderHeight(height));
		}
		let pos = pmmr::insertion_to_pmmr_index(height);
		let header_pmmr = ReadonlyPMMR::at(&self.backend, self.size);
		if let Some(entry) = header_pmmr.get_data(pos) {
			Ok(entry.hash())
		} else {
			Err(Error::Other("get header hash by height".to_string()))
		}
	}

	/// Get the header hash for the head of the header chain based on current MMR state.
	/// Find the last leaf pos based on MMR size and return its header hash.
	pub fn head_hash(&self) -> Result<Hash, Error> {
		if self.size == 0 {
			return Err(Error::Other("MMR empty, no head".to_string()));
		}
		let header_pmmr = ReadonlyPMMR::at(&self.backend, self.size);
		let leaf_pos = pmmr::bintree_rightmost(self.size - 1);
		if let Some(entry) = header_pmmr.get_data(leaf_pos) {
			Ok(entry.hash())
		} else {
			Err(Error::Other("failed to find head hash".to_string()))
		}
	}

	/// Get the first header with all output and kernel mmrs > provided
	pub fn get_first_header_with(
		&self,
		output_pos: u64,
		kernel_pos: u64,
		from_height: u64,
		store: Arc<store::ChainStore>,
	) -> Option<BlockHeader> {
		let mut cur_height = pmmr::round_up_to_leaf_pos(from_height);
		let header_pmmr = ReadonlyPMMR::at(&self.backend, self.size);
		let mut candidate: Option<BlockHeader> = None;
		while let Some(header_entry) = header_pmmr.get_data(cur_height) {
			if let Ok(bh) = store.get_block_header(&header_entry.hash()) {
				if bh.output_mmr_size <= output_pos && bh.kernel_mmr_size <= kernel_pos {
					candidate = Some(bh)
				} else {
					return candidate;
				}
			}
			cur_height = pmmr::round_up_to_leaf_pos(cur_height + 1);
		}
		None
	}
}

/// An easy to manipulate structure holding the 3 MMRs necessary to
/// validate blocks and capturing the output set, associated rangeproofs and the
/// kernels. Also handles the index of Commitments to positions in the
/// output and rangeproof MMRs.
///
/// Note that the index is never authoritative, only the trees are
/// guaranteed to indicate whether an output is spent or not. The index
/// may have commitments that have already been spent, even with
/// pruning enabled.
pub struct TxHashSet {
	output_pmmr_h: PMMRHandle<OutputIdentifier>,
	rproof_pmmr_h: PMMRHandle<RangeProof>,
	kernel_pmmr_h: PMMRHandle<TxKernel>,

	bitmap_accumulator: BitmapAccumulator,

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
		let output_pmmr_h = PMMRHandle::new(
			Path::new(&root_dir)
				.join(TXHASHSET_SUBDIR)
				.join(OUTPUT_SUBDIR),
			true,
			ProtocolVersion(1),
			header,
		)?;

		let rproof_pmmr_h = PMMRHandle::new(
			Path::new(&root_dir)
				.join(TXHASHSET_SUBDIR)
				.join(RANGE_PROOF_SUBDIR),
			true,
			ProtocolVersion(1),
			header,
		)?;

		// Initialize the bitmap accumulator from the current output PMMR.
		let bitmap_accumulator = TxHashSet::bitmap_accumulator(&output_pmmr_h)?;

		let mut maybe_kernel_handle: Option<PMMRHandle<TxKernel>> = None;
		let versions = vec![ProtocolVersion(2), ProtocolVersion(1)];
		for version in versions {
			let handle = PMMRHandle::new(
				Path::new(&root_dir)
					.join(TXHASHSET_SUBDIR)
					.join(KERNEL_SUBDIR),
				false, // not prunable
				version,
				None,
			)?;
			if handle.size == 0 {
				debug!(
					"attempting to open (empty) kernel PMMR using {:?} - SUCCESS",
					version
				);
				maybe_kernel_handle = Some(handle);
				break;
			}
			let kernel: Option<TxKernel> = ReadonlyPMMR::at(&handle.backend, 1).get_data(0);
			if let Some(kernel) = kernel {
				if kernel.verify().is_ok() {
					debug!(
						"attempting to open kernel PMMR using {:?} - SUCCESS",
						version
					);
					maybe_kernel_handle = Some(handle);
					break;
				} else {
					debug!(
						"attempting to open kernel PMMR using {:?} - FAIL (verify failed)",
						version
					);
				}
			} else {
				debug!(
					"attempting to open kernel PMMR using {:?} - FAIL (read failed)",
					version
				);
			}
		}
		if let Some(kernel_pmmr_h) = maybe_kernel_handle {
			Ok(TxHashSet {
				output_pmmr_h,
				rproof_pmmr_h,
				kernel_pmmr_h,
				bitmap_accumulator,
				commit_index,
			})
		} else {
			Err(Error::TxHashSetErr(
				"failed to open kernel PMMR".to_string(),
			))
		}
	}

	// Build a new bitmap accumulator for the provided output PMMR.
	fn bitmap_accumulator(
		pmmr_h: &PMMRHandle<OutputIdentifier>,
	) -> Result<BitmapAccumulator, Error> {
		let pmmr = ReadonlyPMMR::at(&pmmr_h.backend, pmmr_h.size);
		let nbits = pmmr::n_leaves(pmmr_h.size);
		let mut bitmap_accumulator = BitmapAccumulator::new();
		bitmap_accumulator.init(&mut pmmr.leaf_idx_iter(0), nbits)?;
		Ok(bitmap_accumulator)
	}

	/// Close all backend file handles
	pub fn release_backend_files(&mut self) {
		self.output_pmmr_h.backend.release_files();
		self.rproof_pmmr_h.backend.release_files();
		self.kernel_pmmr_h.backend.release_files();
	}

	/// Check if an output is unspent.
	/// We look in the index to find the output MMR pos.
	/// Then we check the entry in the output MMR and confirm the hash matches.
	pub fn get_unspent(
		&self,
		commit: Commitment,
	) -> Result<Option<(OutputIdentifier, CommitPos)>, Error> {
		match self.commit_index.get_output_pos_height(&commit) {
			Ok(Some(pos1)) => {
				let output_pmmr: ReadonlyPMMR<'_, OutputIdentifier, _> =
					ReadonlyPMMR::at(&self.output_pmmr_h.backend, self.output_pmmr_h.size);
				if let Some(out) = output_pmmr.get_data(pos1.pos - 1) {
					if out.commitment() == commit {
						Ok(Some((out, pos1)))
					} else {
						Ok(None)
					}
				} else {
					Ok(None)
				}
			}
			Ok(None) => Ok(None),
			Err(e) => Err(Error::StoreErr(e, "txhashset unspent check".to_string())),
		}
	}

	/// returns the last N nodes inserted into the tree (i.e. the 'bottom'
	/// nodes at level 0
	/// TODO: These need to return the actual data from the flat-files instead
	/// of hashes now
	pub fn last_n_output(&self, distance: u64) -> Vec<(Hash, OutputIdentifier)> {
		ReadonlyPMMR::at(&self.output_pmmr_h.backend, self.output_pmmr_h.size)
			.get_last_n_insertions(distance)
	}

	/// as above, for range proofs
	pub fn last_n_rangeproof(&self, distance: u64) -> Vec<(Hash, RangeProof)> {
		ReadonlyPMMR::at(&self.rproof_pmmr_h.backend, self.rproof_pmmr_h.size)
			.get_last_n_insertions(distance)
	}

	/// as above, for kernels
	pub fn last_n_kernel(&self, distance: u64) -> Vec<(Hash, TxKernel)> {
		ReadonlyPMMR::at(&self.kernel_pmmr_h.backend, self.kernel_pmmr_h.size)
			.get_last_n_insertions(distance)
	}

	/// Efficient view into the kernel PMMR based on size in header.
	pub fn kernel_pmmr_at(
		&self,
		header: &BlockHeader,
	) -> ReadonlyPMMR<TxKernel, PMMRBackend<TxKernel>> {
		ReadonlyPMMR::at(&self.kernel_pmmr_h.backend, header.kernel_mmr_size)
	}

	/// Efficient view into the output PMMR based on size in header.
	pub fn output_pmmr_at(
		&self,
		header: &BlockHeader,
	) -> ReadonlyPMMR<OutputIdentifier, PMMRBackend<OutputIdentifier>> {
		ReadonlyPMMR::at(&self.output_pmmr_h.backend, header.output_mmr_size)
	}

	/// Efficient view into the rangeproof PMMR based on size in header.
	pub fn rangeproof_pmmr_at(
		&self,
		header: &BlockHeader,
	) -> ReadonlyPMMR<RangeProof, PMMRBackend<RangeProof>> {
		ReadonlyPMMR::at(&self.rproof_pmmr_h.backend, header.output_mmr_size)
	}

	/// Convenience function to query the db for a header by its hash.
	pub fn get_block_header(&self, hash: &Hash) -> Result<BlockHeader, Error> {
		Ok(self.commit_index.get_block_header(&hash)?)
	}

	/// returns outputs from the given pmmr index up to the
	/// specified limit. Also returns the last index actually populated
	/// max index is the last PMMR index to consider, not leaf index
	pub fn outputs_by_pmmr_index(
		&self,
		start_index: u64,
		max_count: u64,
		max_index: Option<u64>,
	) -> (u64, Vec<OutputIdentifier>) {
		ReadonlyPMMR::at(&self.output_pmmr_h.backend, self.output_pmmr_h.size)
			.elements_from_pmmr_index(start_index, max_count, max_index)
	}

	/// As above, for rangeproofs
	pub fn rangeproofs_by_pmmr_index(
		&self,
		start_index: u64,
		max_count: u64,
		max_index: Option<u64>,
	) -> (u64, Vec<RangeProof>) {
		ReadonlyPMMR::at(&self.rproof_pmmr_h.backend, self.rproof_pmmr_h.size)
			.elements_from_pmmr_index(start_index, max_count, max_index)
	}

	/// size of output mmr
	pub fn output_mmr_size(&self) -> u64 {
		self.output_pmmr_h.size
	}

	/// size of kernel mmr
	pub fn kernel_mmr_size(&self) -> u64 {
		self.kernel_pmmr_h.size
	}

	/// size of rangeproof mmr (can differ from output mmr size during PIBD sync)
	pub fn rangeproof_mmr_size(&self) -> u64 {
		self.rproof_pmmr_h.size
	}

	/// Find a kernel with a given excess. Work backwards from `max_index` to `min_index`
	/// NOTE: this linear search over all kernel history can be VERY expensive
	/// public API access to this method should be limited
	pub fn find_kernel(
		&self,
		excess: &Commitment,
		min_index: Option<u64>,
		max_index: Option<u64>,
	) -> Option<(TxKernel, u64)> {
		let min_index = min_index.unwrap_or(1);
		let max_index = max_index.unwrap_or(self.kernel_pmmr_h.size);

		let pmmr = ReadonlyPMMR::at(&self.kernel_pmmr_h.backend, self.kernel_pmmr_h.size);
		let mut index = max_index + 1;
		while index > min_index {
			index -= 1;
			if let Some(kernel) = pmmr.get_data(index - 1) {
				if &kernel.excess == excess {
					return Some((kernel, index));
				}
			}
		}
		None
	}

	/// Get MMR roots.
	pub fn roots(&self) -> TxHashSetRoots {
		let output_pmmr = ReadonlyPMMR::at(&self.output_pmmr_h.backend, self.output_pmmr_h.size);
		let rproof_pmmr = ReadonlyPMMR::at(&self.rproof_pmmr_h.backend, self.rproof_pmmr_h.size);
		let kernel_pmmr = ReadonlyPMMR::at(&self.kernel_pmmr_h.backend, self.kernel_pmmr_h.size);

		TxHashSetRoots {
			output_roots: OutputRoots {
				pmmr_root: output_pmmr.root().expect("no root, invalid tree"),
				bitmap_root: self.bitmap_accumulator.root(),
			},
			rproof_root: rproof_pmmr.root().expect("no root, invalid tree"),
			kernel_root: kernel_pmmr.root().expect("no root, invalid tree"),
		}
	}

	/// Return Commit's MMR position
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		Ok(self.commit_index.get_output_pos(&commit)?)
	}

	/// build a new merkle proof for the given output commitment
	pub fn merkle_proof(&mut self, commit: Commitment) -> Result<MerkleProof, Error> {
		let pos0 = self.commit_index.get_output_pos(&commit)?;
		PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.size)
			.merkle_proof(pos0)
			.map_err(|_| Error::MerkleProof)
	}

	/// Compact the MMR data files and flush the rm logs
	pub fn compact(
		&mut self,
		horizon_header: &BlockHeader,
		batch: &Batch<'_>,
	) -> Result<(), Error> {
		debug!("txhashset: starting compaction...");

		let head_header = batch.head_header()?;

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

	/// (Re)build the NRD kernel_pos index based on 2 weeks of recent kernel history.
	pub fn init_recent_kernel_pos_index(
		&self,
		header_pmmr: &PMMRHandle<BlockHeader>,
		batch: &Batch<'_>,
	) -> Result<(), Error> {
		let head = batch.head()?;
		let cutoff = head.height.saturating_sub(WEEK_HEIGHT * 2);
		let cutoff_hash = header_pmmr.get_header_hash_by_height(cutoff)?;
		let cutoff_header = batch.get_block_header(&cutoff_hash)?;
		self.verify_kernel_pos_index(&cutoff_header, header_pmmr, batch, None, None)
	}

	/// Verify and (re)build the NRD kernel_pos index from the provided header onwards.
	pub fn verify_kernel_pos_index(
		&self,
		from_header: &BlockHeader,
		header_pmmr: &PMMRHandle<BlockHeader>,
		batch: &Batch<'_>,
		status: Option<Arc<SyncState>>,
		stop_state: Option<Arc<StopState>>,
	) -> Result<(), Error> {
		if !global::is_nrd_enabled() {
			return Ok(());
		}

		let now = Instant::now();
		let kernel_index = store::nrd_recent_kernel_index();
		kernel_index.clear(batch)?;

		let prev_size = if from_header.height == 0 {
			0
		} else {
			let prev_header = batch.get_previous_header(&from_header)?;
			prev_header.kernel_mmr_size
		};

		debug!(
			"verify_kernel_pos_index: header: {} at {}, prev kernel_mmr_size: {}",
			from_header.hash(),
			from_header.height,
			prev_size,
		);

		let kernel_pmmr = ReadonlyPMMR::at(&self.kernel_pmmr_h.backend, self.kernel_pmmr_h.size);

		let mut current_pos = prev_size + 1;
		let mut current_header = from_header.clone();
		let mut count = 0;
		let total = pmmr::n_leaves(self.kernel_pmmr_h.size);
		let mut applied = 0;
		while current_pos <= self.kernel_pmmr_h.size {
			if pmmr::is_leaf(current_pos - 1) {
				if let Some(kernel) = kernel_pmmr.get_data(current_pos - 1) {
					match kernel.features {
						KernelFeatures::NoRecentDuplicate { .. } => {
							while current_pos > current_header.kernel_mmr_size {
								let hash = header_pmmr
									.get_header_hash_by_height(current_header.height + 1)?;
								current_header = batch.get_block_header(&hash)?;
							}
							let new_pos = CommitPos {
								pos: current_pos,
								height: current_header.height,
							};
							apply_kernel_rules(&kernel, new_pos, batch)?;
							count += 1;
						}
						_ => {}
					}
				}
				applied += 1;
				if let Some(ref s) = status {
					if total % applied == 10000 {
						s.on_setup(None, None, Some(applied), Some(total));
					}
				}
			}
			if let Some(ref s) = stop_state {
				if s.is_stopped() {
					return Ok(());
				}
			}

			current_pos += 1;
		}

		debug!(
			"verify_kernel_pos_index: pushed {} entries to the index, took {}s",
			count,
			now.elapsed().as_secs(),
		);
		Ok(())
	}

	/// (Re)build the output_pos index to be consistent with the current UTXO set.
	/// Remove any "stale" index entries that do not correspond to outputs in the UTXO set.
	/// Add any missing index entries based on UTXO set.
	pub fn init_output_pos_index(
		&self,
		header_pmmr: &PMMRHandle<BlockHeader>,
		batch: &Batch<'_>,
	) -> Result<(), Error> {
		let now = Instant::now();

		let output_pmmr = ReadonlyPMMR::at(&self.output_pmmr_h.backend, self.output_pmmr_h.size);

		// Iterate over the current output_pos index, removing any entries that
		// do not point to to the expected output.
		let mut removed_count = 0;
		for (key, pos1) in batch.output_pos_iter()? {
			let pos0 = pos1.pos - 1;
			if let Some(out) = output_pmmr.get_data(pos0) {
				if let Ok(pos0_via_mmr) = batch.get_output_pos(&out.commitment()) {
					// If the pos matches and the index key matches the commitment
					// then keep the entry, other we want to clean it up.
					if pos0 == pos0_via_mmr
						&& batch.is_match_output_pos_key(&key, &out.commitment())
					{
						continue;
					}
				}
			}
			batch.delete(&key)?;
			removed_count += 1;
		}
		debug!(
			"init_output_pos_index: removed {} stale index entries",
			removed_count
		);

		let mut outputs_pos: Vec<(Commitment, u64)> = vec![];
		for pos0 in output_pmmr.leaf_pos_iter() {
			if let Some(out) = output_pmmr.get_data(pos0) {
				outputs_pos.push((out.commit, 1 + pos0));
			}
		}

		debug!("init_output_pos_index: {} utxos", outputs_pos.len());

		outputs_pos.retain(|x| {
			batch
				.get_output_pos_height(&x.0)
				.map(|p| p.is_none())
				.unwrap_or(true)
		});

		debug!(
			"init_output_pos_index: {} utxos with missing index entries",
			outputs_pos.len()
		);

		if outputs_pos.is_empty() {
			return Ok(());
		}

		let total_outputs = outputs_pos.len();
		let max_height = batch.head()?.height;

		let mut i = 0;
		for search_height in 0..max_height {
			let hash = header_pmmr.get_header_hash_by_height(search_height + 1)?;
			let h = batch.get_block_header(&hash)?;
			while i < total_outputs {
				let (commit, pos1) = outputs_pos[i];
				if pos1 > h.output_mmr_size {
					break;
				}
				batch.save_output_pos_height(
					&commit,
					CommitPos {
						pos: pos1,
						height: h.height,
					},
				)?;
				i += 1;
			}
		}
		debug!(
			"init_output_pos_index: added entries for {} utxos, took {}s",
			total_outputs,
			now.elapsed().as_secs(),
		);
		Ok(())
	}
}

/// Starts a new unit of work to extend (or rewind) the chain with additional
/// blocks. Accepts a closure that will operate within that unit of work.
/// The closure has access to an Extension object that allows the addition
/// of blocks to the txhashset and the checking of the current tree roots.
///
/// The unit of work is always discarded (always rollback) as this is read-only.
pub fn extending_readonly<F, T>(
	handle: &mut PMMRHandle<BlockHeader>,
	trees: &mut TxHashSet,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut ExtensionPair<'_>, &Batch<'_>) -> Result<T, Error>,
{
	let commit_index = trees.commit_index.clone();
	let batch = commit_index.batch()?;

	trace!("Starting new txhashset (readonly) extension.");

	let head = batch.head()?;
	let header_head = batch.header_head()?;

	let res = {
		let header_pmmr = PMMR::at(&mut handle.backend, handle.size);
		let mut header_extension = HeaderExtension::new(header_pmmr, header_head);
		let mut extension = Extension::new(trees, head);
		let mut extension_pair = ExtensionPair {
			header_extension: &mut header_extension,
			extension: &mut extension,
		};
		inner(&mut extension_pair, &batch)
	};

	trace!("Rollbacking txhashset (readonly) extension.");

	handle.backend.discard();

	trees.output_pmmr_h.backend.discard();
	trees.rproof_pmmr_h.backend.discard();
	trees.kernel_pmmr_h.backend.discard();

	trace!("TxHashSet (readonly) extension done.");

	res
}

/// Readonly view on the UTXO set.
/// Based on the current txhashset output_pmmr.
pub fn utxo_view<F, T>(
	handle: &PMMRHandle<BlockHeader>,
	trees: &TxHashSet,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&UTXOView<'_>, &Batch<'_>) -> Result<T, Error>,
{
	let res: Result<T, Error>;
	{
		let header_pmmr = ReadonlyPMMR::at(&handle.backend, handle.size);
		let output_pmmr = ReadonlyPMMR::at(&trees.output_pmmr_h.backend, trees.output_pmmr_h.size);
		let rproof_pmmr = ReadonlyPMMR::at(&trees.rproof_pmmr_h.backend, trees.rproof_pmmr_h.size);

		// Create a new batch here to pass into the utxo_view.
		// Discard it (rollback) after we finish with the utxo_view.
		let batch = trees.commit_index.batch()?;
		let utxo = UTXOView::new(header_pmmr, output_pmmr, rproof_pmmr);
		res = inner(&utxo, &batch);
	}
	res
}

/// Rewindable (but still readonly) view on the kernel MMR.
/// The underlying backend is readonly. But we permit the PMMR to be "rewound"
/// via size.
/// We create a new db batch for this view and discard it (rollback)
/// when we are done with the view.
pub fn rewindable_kernel_view<F, T>(trees: &TxHashSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&mut RewindableKernelView<'_>, &Batch<'_>) -> Result<T, Error>,
{
	let res: Result<T, Error>;
	{
		let kernel_pmmr =
			RewindablePMMR::at(&trees.kernel_pmmr_h.backend, trees.kernel_pmmr_h.size);

		// Create a new batch here to pass into the kernel_view.
		// Discard it (rollback) after we finish with the kernel_view.
		let batch = trees.commit_index.batch()?;
		let header = batch.head_header()?;
		let mut view = RewindableKernelView::new(kernel_pmmr, header);
		res = inner(&mut view, &batch);
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
	header_pmmr: &'a mut PMMRHandle<BlockHeader>,
	trees: &'a mut TxHashSet,
	batch: &'a mut Batch<'_>,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut ExtensionPair<'_>, &Batch<'_>) -> Result<T, Error>,
{
	let sizes: (u64, u64, u64);
	let res: Result<T, Error>;
	let rollback: bool;
	let bitmap_accumulator: BitmapAccumulator;

	let head = batch.head()?;
	let header_head = batch.header_head()?;

	// create a child transaction so if the state is rolled back by itself, all
	// index saving can be undone
	let child_batch = batch.child()?;
	{
		trace!("Starting new txhashset extension.");

		let header_pmmr = PMMR::at(&mut header_pmmr.backend, header_pmmr.size);
		let mut header_extension = HeaderExtension::new(header_pmmr, header_head);
		let mut extension = Extension::new(trees, head);
		let mut extension_pair = ExtensionPair {
			header_extension: &mut header_extension,
			extension: &mut extension,
		};
		res = inner(&mut extension_pair, &child_batch);

		rollback = extension_pair.extension.rollback;
		sizes = extension_pair.extension.sizes();
		bitmap_accumulator = extension_pair.extension.bitmap_accumulator.clone();
	}

	// During an extension we do not want to modify the header_extension (and only read from it).
	// So make sure we discard any changes to the header MMR backed.
	header_pmmr.backend.discard();

	match res {
		Err(e) => {
			debug!("Error returned, discarding txhashset extension: {}", e);
			trees.output_pmmr_h.backend.discard();
			trees.rproof_pmmr_h.backend.discard();
			trees.kernel_pmmr_h.backend.discard();
			Err(e)
		}
		Ok(r) => {
			if rollback {
				trace!("Rollbacking txhashset extension. sizes {:?}", sizes);
				trees.output_pmmr_h.backend.discard();
				trees.rproof_pmmr_h.backend.discard();
				trees.kernel_pmmr_h.backend.discard();
			} else {
				trace!("Committing txhashset extension. sizes {:?}", sizes);
				child_batch.commit()?;
				trees.output_pmmr_h.backend.sync()?;
				trees.rproof_pmmr_h.backend.sync()?;
				trees.kernel_pmmr_h.backend.sync()?;
				trees.output_pmmr_h.size = sizes.0;
				trees.rproof_pmmr_h.size = sizes.1;
				trees.kernel_pmmr_h.size = sizes.2;

				// Update our bitmap_accumulator based on our extension
				trees.bitmap_accumulator = bitmap_accumulator;
			}

			trace!("TxHashSet extension done.");
			Ok(r)
		}
	}
}

/// Start a new readonly header MMR extension.
/// This MMR can be extended individually beyond the other (output, rangeproof and kernel) MMRs
/// to allow headers to be validated before we receive the full block data.
pub fn header_extending_readonly<'a, F, T>(
	handle: &'a mut PMMRHandle<BlockHeader>,
	store: &ChainStore,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut HeaderExtension<'_>, &Batch<'_>) -> Result<T, Error>,
{
	let batch = store.batch()?;

	let head = match handle.head_hash() {
		Ok(hash) => {
			let header = batch.get_block_header(&hash)?;
			Tip::from_header(&header)
		}
		Err(_) => Tip::default(),
	};

	let pmmr = PMMR::at(&mut handle.backend, handle.size);
	let mut extension = HeaderExtension::new(pmmr, head);
	let res = inner(&mut extension, &batch);

	handle.backend.discard();

	res
}

/// Start a new header MMR unit of work.
/// This MMR can be extended individually beyond the other (output, rangeproof and kernel) MMRs
/// to allow headers to be validated before we receive the full block data.
pub fn header_extending<'a, F, T>(
	handle: &'a mut PMMRHandle<BlockHeader>,
	batch: &'a mut Batch<'_>,
	inner: F,
) -> Result<T, Error>
where
	F: FnOnce(&mut HeaderExtension<'_>, &Batch<'_>) -> Result<T, Error>,
{
	let size: u64;
	let res: Result<T, Error>;
	let rollback: bool;

	// create a child transaction so if the state is rolled back by itself, all
	// index saving can be undone
	let child_batch = batch.child()?;

	let head = match handle.head_hash() {
		Ok(hash) => {
			let header = child_batch.get_block_header(&hash)?;
			Tip::from_header(&header)
		}
		Err(_) => Tip::default(),
	};

	{
		let pmmr = PMMR::at(&mut handle.backend, handle.size);
		let mut extension = HeaderExtension::new(pmmr, head);
		res = inner(&mut extension, &child_batch);

		rollback = extension.rollback;
		size = extension.size();
	}

	match res {
		Err(e) => {
			handle.backend.discard();
			Err(e)
		}
		Ok(r) => {
			if rollback {
				handle.backend.discard();
			} else {
				child_batch.commit()?;
				handle.backend.sync()?;
				handle.size = size;
			}
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
}

impl<'a> HeaderExtension<'a> {
	fn new(
		pmmr: PMMR<'a, BlockHeader, PMMRBackend<BlockHeader>>,
		head: Tip,
	) -> HeaderExtension<'a> {
		HeaderExtension {
			head,
			pmmr,
			rollback: false,
		}
	}

	/// Get the header hash for the specified pos from the underlying MMR backend.
	fn get_header_hash(&self, pos0: u64) -> Option<Hash> {
		self.pmmr.get_data(pos0).map(|x| x.hash())
	}

	/// The head representing the furthest extent of the current extension.
	pub fn head(&self) -> Tip {
		self.head.clone()
	}

	/// Get header hash by height.
	/// Based on current header MMR.
	pub fn get_header_hash_by_height(&self, height: u64) -> Option<Hash> {
		let pos = pmmr::insertion_to_pmmr_index(height);
		self.get_header_hash(pos)
	}

	/// Get the header at the specified height based on the current state of the header extension.
	/// Derives the MMR pos from the height (insertion index) and retrieves the header hash.
	/// Looks the header up in the db by hash.
	pub fn get_header_by_height(
		&self,
		height: u64,
		batch: &Batch<'_>,
	) -> Result<BlockHeader, Error> {
		if let Some(hash) = self.get_header_hash_by_height(height) {
			Ok(batch.get_block_header(&hash)?)
		} else {
			Err(Error::Other("get header by height".to_string()))
		}
	}

	/// Compares the provided header to the header in the header MMR at that height.
	/// If these match we know the header is on the current chain.
	pub fn is_on_current_chain<T: Into<Tip>>(
		&self,
		t: T,
		batch: &Batch<'_>,
	) -> Result<bool, Error> {
		let t = t.into();
		if t.height > self.head.height {
			return Ok(false);
		}
		let chain_header = self.get_header_by_height(t.height, batch)?;
		Ok(chain_header.hash() == t.hash())
	}

	/// Force the rollback of this extension, no matter the result.
	pub fn force_rollback(&mut self) {
		self.rollback = true;
	}

	/// Apply a new header to the header MMR extension.
	/// This may be either the header MMR or the sync MMR depending on the
	/// extension.
	pub fn apply_header(&mut self, header: &BlockHeader) -> Result<(), Error> {
		self.pmmr.push(header).map_err(&Error::TxHashSetErr)?;
		self.head = Tip::from_header(header);
		Ok(())
	}

	/// Rewind the header extension to the specified header.
	/// Note the close relationship between header height and insertion index.
	pub fn rewind(&mut self, header: &BlockHeader) -> Result<(), Error> {
		debug!(
			"Rewind header extension to {} at {} from {} at {}",
			header.hash(),
			header.height,
			self.head.hash(),
			self.head.height,
		);

		let header_pos = 1 + pmmr::insertion_to_pmmr_index(header.height);
		self.pmmr
			.rewind(header_pos, &Bitmap::create())
			.map_err(&Error::TxHashSetErr)?;

		// Update our head to reflect the header we rewound to.
		self.head = Tip::from_header(header);

		Ok(())
	}

	/// The size of the header MMR.
	pub fn size(&self) -> u64 {
		self.pmmr.unpruned_size()
	}

	/// The root of the header MMR for convenience.
	pub fn root(&self) -> Result<Hash, Error> {
		Ok(self.pmmr.root().map_err(|_| Error::InvalidRoot)?)
	}

	/// Validate the prev_root of the header against the root of the current header MMR.
	pub fn validate_root(&self, header: &BlockHeader) -> Result<(), Error> {
		// If we are validating the genesis block then we have no prev_root.
		// So we are done here.
		if header.height == 0 {
			return Ok(());
		}
		if self.root()? != header.prev_root {
			Err(Error::InvalidRoot)
		} else {
			Ok(())
		}
	}
}

/// An extension "pair" consisting of a txhashet extension (outputs, rangeproofs, kernels)
/// and the associated header extension.
pub struct ExtensionPair<'a> {
	/// The header extension.
	pub header_extension: &'a mut HeaderExtension<'a>,
	/// The txhashset extension.
	pub extension: &'a mut Extension<'a>,
}

/// Allows the application of new blocks on top of the txhashset in a
/// reversible manner within a unit of work provided by the `extending`
/// function.
pub struct Extension<'a> {
	head: Tip,

	output_pmmr: PMMR<'a, OutputIdentifier, PMMRBackend<OutputIdentifier>>,
	rproof_pmmr: PMMR<'a, RangeProof, PMMRBackend<RangeProof>>,
	kernel_pmmr: PMMR<'a, TxKernel, PMMRBackend<TxKernel>>,

	bitmap_accumulator: BitmapAccumulator,
	bitmap_cache: Bitmap,

	/// Rollback flag.
	rollback: bool,
}

impl<'a> Committed for Extension<'a> {
	fn inputs_committed(&self) -> Vec<Commitment> {
		vec![]
	}

	fn outputs_committed(&self) -> Vec<Commitment> {
		let mut commitments = vec![];
		for pos0 in self.output_pmmr.leaf_pos_iter() {
			if let Some(out) = self.output_pmmr.get_data(pos0) {
				commitments.push(out.commit);
			}
		}
		commitments
	}

	fn kernels_committed(&self) -> Vec<Commitment> {
		let mut commitments = vec![];
		for n in 0..self.kernel_pmmr.unpruned_size() {
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
	fn new(trees: &'a mut TxHashSet, head: Tip) -> Extension<'a> {
		Extension {
			head,
			output_pmmr: PMMR::at(&mut trees.output_pmmr_h.backend, trees.output_pmmr_h.size),
			rproof_pmmr: PMMR::at(&mut trees.rproof_pmmr_h.backend, trees.rproof_pmmr_h.size),
			kernel_pmmr: PMMR::at(&mut trees.kernel_pmmr_h.backend, trees.kernel_pmmr_h.size),
			bitmap_accumulator: trees.bitmap_accumulator.clone(),
			bitmap_cache: trees
				.bitmap_accumulator
				.as_bitmap()
				.unwrap_or(Bitmap::create()),
			rollback: false,
		}
	}

	/// The head representing the furthest extent of the current extension.
	pub fn head(&self) -> Tip {
		self.head.clone()
	}

	/// Build a view of the current UTXO set based on the output PMMR
	/// and the provided header extension.
	pub fn utxo_view(&'a self, header_ext: &'a HeaderExtension<'a>) -> UTXOView<'a> {
		UTXOView::new(
			header_ext.pmmr.readonly_pmmr(),
			self.output_readonly_pmmr(),
			self.rproof_readonly_pmmr(),
		)
	}

	/// Readonly view of our output data.
	pub fn output_readonly_pmmr(
		&self,
	) -> ReadonlyPMMR<OutputIdentifier, PMMRBackend<OutputIdentifier>> {
		self.output_pmmr.readonly_pmmr()
	}

	/// Take a snapshot of our bitmap accumulator
	pub fn bitmap_accumulator(&self) -> BitmapAccumulator {
		self.bitmap_accumulator.clone()
	}

	/// Readonly view of our bitmap accumulator data.
	pub fn bitmap_readonly_pmmr(&self) -> ReadonlyPMMR<BitmapChunk, VecBackend<BitmapChunk>> {
		self.bitmap_accumulator.readonly_pmmr()
	}

	/// Readonly view of our rangeproof data.
	pub fn rproof_readonly_pmmr(&self) -> ReadonlyPMMR<RangeProof, PMMRBackend<RangeProof>> {
		self.rproof_pmmr.readonly_pmmr()
	}

	/// Reset prune lists
	pub fn reset_prune_lists(&mut self) {
		self.output_pmmr.reset_prune_list();
		self.rproof_pmmr.reset_prune_list();
	}

	/// Apply a new block to the current txhashet extension (output, rangeproof, kernel MMRs).
	/// Returns a vec of commit_pos representing the pos and height of the outputs spent
	/// by this block.
	pub fn apply_block(
		&mut self,
		b: &Block,
		header_ext: &HeaderExtension<'_>,
		batch: &Batch<'_>,
	) -> Result<(), Error> {
		let mut affected_pos = vec![];

		// Apply the output to the output and rangeproof MMRs.
		// Add pos to affected_pos to update the accumulator later on.
		// Add the new output to the output_pos index.
		for out in b.outputs() {
			let pos = self.apply_output(out, batch)?;
			affected_pos.push(pos);
			batch.save_output_pos_height(
				&out.commitment(),
				CommitPos {
					pos,
					height: b.header.height,
				},
			)?;
		}

		// Use our utxo_view to identify outputs being spent by block inputs.
		// Apply inputs to remove spent outputs from the output and rangeproof MMRs.
		// Add spent_pos to affected_pos to update the accumulator later on.
		// Remove the spent outputs from the output_pos index.
		let spent = self
			.utxo_view(header_ext)
			.validate_inputs(&b.inputs(), batch)?;
		for (out, pos) in &spent {
			self.apply_input(out.commitment(), *pos)?;
			affected_pos.push(pos.pos);
			batch.delete_output_pos_height(&out.commitment())?;
		}

		// Update the spent index with spent pos.
		let spent: Vec<_> = spent.into_iter().map(|(_, pos)| pos).collect();
		batch.save_spent_index(&b.hash(), &spent)?;

		// Apply the kernels to the kernel MMR.
		// Note: This validates and NRD relative height locks via the "recent" kernel index.
		self.apply_kernels(b.kernels(), b.header.height, batch)?;

		// Update our BitmapAccumulator based on affected outputs (both spent and created).
		self.apply_to_bitmap_accumulator(&affected_pos)?;

		// Update the head of the extension to reflect the block we just applied.
		self.head = Tip::from_header(&b.header);

		Ok(())
	}

	fn apply_to_bitmap_accumulator(&mut self, output_pos: &[u64]) -> Result<(), Error> {
		// NOTE: 1-based output_pos shouldn't have 0 in it (but does)
		let mut output_idx: Vec<_> = output_pos
			.iter()
			.map(|x| pmmr::n_leaves(*x).saturating_sub(1))
			.collect();
		output_idx.sort_unstable();
		let min_idx = output_idx.first().cloned().unwrap_or(0);
		let size = pmmr::n_leaves(self.output_pmmr.size);
		self.bitmap_accumulator.apply(
			output_idx,
			self.output_pmmr
				.leaf_idx_iter(BitmapAccumulator::chunk_start_idx(min_idx)),
			size,
		)
	}

	/// Sets the bitmap accumulator (as received during PIBD sync)
	pub fn set_bitmap_accumulator(&mut self, accumulator: BitmapAccumulator) {
		self.bitmap_accumulator = accumulator;
		self.bitmap_cache = self
			.bitmap_accumulator
			.as_bitmap()
			.unwrap_or(Bitmap::create());
	}

	// Prune output and rangeproof PMMRs based on provided pos.
	// Input is not valid if we cannot prune successfully.
	fn apply_input(&mut self, commit: Commitment, pos: CommitPos) -> Result<(), Error> {
		match self.output_pmmr.prune(pos.pos - 1) {
			Ok(true) => {
				self.rproof_pmmr
					.prune(pos.pos - 1)
					.map_err(Error::TxHashSetErr)?;
				Ok(())
			}
			Ok(false) => Err(Error::AlreadySpent(commit)),
			Err(e) => Err(Error::TxHashSetErr(e)),
		}
	}

	fn apply_output(&mut self, out: &Output, batch: &Batch<'_>) -> Result<u64, Error> {
		let commit = out.commitment();

		if let Ok(pos0) = batch.get_output_pos(&commit) {
			if let Some(out_mmr) = self.output_pmmr.get_data(pos0) {
				if out_mmr.commitment() == commit {
					return Err(Error::DuplicateCommitment(commit));
				}
			}
		}
		// push the new output to the MMR.
		let output_pos = self
			.output_pmmr
			.push(&out.identifier())
			.map_err(&Error::TxHashSetErr)?;

		// push the rangeproof to the MMR.
		let rproof_pos = self
			.rproof_pmmr
			.push(&out.proof())
			.map_err(&Error::TxHashSetErr)?;

		// The output and rproof MMRs should be exactly the same size
		// and we should have inserted to both in exactly the same pos.
		{
			if self.output_pmmr.unpruned_size() != self.rproof_pmmr.unpruned_size() {
				return Err(Error::Other(
					"output vs rproof MMRs different sizes".to_string(),
				));
			}

			if output_pos != rproof_pos {
				return Err(Error::Other(
					"output vs rproof MMRs different pos".to_string(),
				));
			}
		}
		Ok(1 + output_pos)
	}

	/// Once the PIBD set is downloaded, we need to ensure that the respective leaf sets
	/// match the bitmap (particularly in the case of outputs being spent after a PIBD catch-up)
	pub fn update_leaf_sets(&mut self, bitmap: &Bitmap) -> Result<(), Error> {
		let flipped = bitmap.flip(0..bitmap.maximum().unwrap() as u64 + 1);
		for spent_pmmr_index in flipped.iter() {
			let pos0 = pmmr::insertion_to_pmmr_index(spent_pmmr_index.into());
			self.output_pmmr.remove_from_leaf_set(pos0);
			self.rproof_pmmr.remove_from_leaf_set(pos0);
		}
		Ok(())
	}

	/// Order and sort output segments and hashes, returning an array
	/// of elements that can be applied in order to a pmmr
	fn sort_pmmr_hashes_and_leaves(
		&mut self,
		hash_pos: Vec<u64>,
		leaf_pos: Vec<u64>,
		skip_leaf_position: Option<u64>,
	) -> Vec<OrderedHashLeafNode> {
		// Merge and into single array and sort into insertion order
		let mut ordered_inserts = vec![];
		for (data_index, pos0) in leaf_pos.iter().enumerate() {
			// Don't re-push genesis output, basically
			if skip_leaf_position == Some(*pos0) {
				continue;
			}
			ordered_inserts.push(OrderedHashLeafNode::Leaf(data_index, *pos0));
		}
		for (data_index, pos0) in hash_pos.iter().enumerate() {
			ordered_inserts.push(OrderedHashLeafNode::Hash(data_index, *pos0));
		}
		ordered_inserts.sort();
		ordered_inserts
	}

	/// Apply an output segment to the output PMMR. must be called in order
	/// Sort and apply hashes and leaves within a segment to output pmmr, skipping over
	/// genesis position.
	/// NB: Would like to make this more generic but the hard casting of pmmrs
	/// held by this struct makes it awkward to do so

	pub fn apply_output_segment(
		&mut self,
		segment: Segment<OutputIdentifier>,
	) -> Result<(), Error> {
		let (_sid, hash_pos, hashes, leaf_pos, leaf_data, _proof) = segment.parts();

		// insert either leaves or pruned subtrees as we go
		for insert in self.sort_pmmr_hashes_and_leaves(hash_pos, leaf_pos, Some(0)) {
			match insert {
				OrderedHashLeafNode::Hash(idx, pos0) => {
					if pos0 >= self.output_pmmr.size {
						if self.output_pmmr.size == 1 {
							// All initial outputs are spent up to this hash,
							// Roll back the genesis output
							self.output_pmmr
								.rewind(0, &Bitmap::create())
								.map_err(&Error::TxHashSetErr)?;
						}
						self.output_pmmr
							.push_pruned_subtree(hashes[idx], pos0)
							.map_err(&Error::TxHashSetErr)?;
					}
				}
				OrderedHashLeafNode::Leaf(idx, pos0) => {
					if pos0 == self.output_pmmr.size {
						self.output_pmmr
							.push(&leaf_data[idx])
							.map_err(&Error::TxHashSetErr)?;
					}
					let pmmr_index = pmmr::pmmr_leaf_to_insertion_index(pos0);
					match pmmr_index {
						Some(i) => {
							if !self.bitmap_cache.contains(i as u32) {
								self.output_pmmr.remove_from_leaf_set(pos0);
							}
						}
						None => {}
					};
				}
			}
		}
		Ok(())
	}

	/// Apply a rangeproof segment to the rangeproof PMMR. must be called in order
	/// Sort and apply hashes and leaves within a segment to rangeproof pmmr, skipping over
	/// genesis position.
	pub fn apply_rangeproof_segment(&mut self, segment: Segment<RangeProof>) -> Result<(), Error> {
		let (_sid, hash_pos, hashes, leaf_pos, leaf_data, _proof) = segment.parts();

		// insert either leaves or pruned subtrees as we go
		for insert in self.sort_pmmr_hashes_and_leaves(hash_pos, leaf_pos, Some(0)) {
			match insert {
				OrderedHashLeafNode::Hash(idx, pos0) => {
					if pos0 >= self.rproof_pmmr.size {
						if self.rproof_pmmr.size == 1 {
							// All initial outputs are spent up to this hash,
							// Roll back the genesis output
							self.rproof_pmmr
								.rewind(0, &Bitmap::create())
								.map_err(&Error::TxHashSetErr)?;
						}
						self.rproof_pmmr
							.push_pruned_subtree(hashes[idx], pos0)
							.map_err(&Error::TxHashSetErr)?;
					}
				}
				OrderedHashLeafNode::Leaf(idx, pos0) => {
					if pos0 == self.rproof_pmmr.size {
						self.rproof_pmmr
							.push(&leaf_data[idx])
							.map_err(&Error::TxHashSetErr)?;
					}
					let pmmr_index = pmmr::pmmr_leaf_to_insertion_index(pos0);
					match pmmr_index {
						Some(i) => {
							if !self.bitmap_cache.contains(i as u32) {
								self.rproof_pmmr.remove_from_leaf_set(pos0);
							}
						}
						None => {}
					};
				}
			}
		}
		Ok(())
	}

	/// Apply kernels to the kernel MMR.
	/// Validate any NRD relative height locks via the "recent" kernel index.
	/// Note: This is used for both block processing and tx validation.
	/// In the block processing case we use the block height.
	/// In the tx validation case we use the "next" block height based on current chain head.
	pub fn apply_kernels(
		&mut self,
		kernels: &[TxKernel],
		height: u64,
		batch: &Batch<'_>,
	) -> Result<(), Error> {
		for kernel in kernels {
			let pos = self.apply_kernel(kernel)?;
			let commit_pos = CommitPos { pos, height };
			apply_kernel_rules(kernel, commit_pos, batch)?;
		}
		Ok(())
	}

	/// Apply a kernel segment to the output PMMR. must be called in order
	pub fn apply_kernel_segment(&mut self, segment: Segment<TxKernel>) -> Result<(), Error> {
		let (_sid, _hash_pos, _hashes, leaf_pos, leaf_data, _proof) = segment.parts();
		// Non prunable - insert only leaves (with genesis kernel removedj)
		for insert in self.sort_pmmr_hashes_and_leaves(vec![], leaf_pos, Some(0)) {
			match insert {
				OrderedHashLeafNode::Hash(_, _) => {
					return Err(Error::InvalidSegment(
						"Kernel PMMR is non-prunable, should not have hash data".to_string(),
					)
					.into());
				}
				OrderedHashLeafNode::Leaf(idx, pos0) => {
					if pos0 == self.kernel_pmmr.size {
						self.kernel_pmmr
							.push(&leaf_data[idx])
							.map_err(&Error::TxHashSetErr)?;
					}
				}
			}
		}
		Ok(())
	}

	/// Push kernel onto MMR (hash and data files).
	fn apply_kernel(&mut self, kernel: &TxKernel) -> Result<u64, Error> {
		let pos = self
			.kernel_pmmr
			.push(kernel)
			.map_err(&Error::TxHashSetErr)?;
		Ok(1 + pos)
	}

	/// Build a Merkle proof for the given output and the block
	/// this extension is currently referencing.
	/// Note: this relies on the MMR being stable even after pruning/compaction.
	/// We need the hash of each sibling pos from the pos up to the peak
	/// including the sibling leaf node which may have been removed.
	pub fn merkle_proof<T: AsRef<OutputIdentifier>>(
		&self,
		out_id: T,
		batch: &Batch<'_>,
	) -> Result<MerkleProof, Error> {
		let out_id = out_id.as_ref();
		debug!("txhashset: merkle_proof: output: {:?}", out_id.commit);
		// then calculate the Merkle Proof based on the known pos
		let pos0 = batch.get_output_pos(&out_id.commit)?;
		let merkle_proof = self
			.output_pmmr
			.merkle_proof(pos0)
			.map_err(&Error::TxHashSetErr)?;

		Ok(merkle_proof)
	}

	/// Saves a snapshot of the output and rangeproof MMRs to disk.
	/// Specifically - saves a snapshot of the utxo file, tagged with
	/// the block hash as filename suffix.
	/// Needed for fast-sync (utxo file needs to be rewound before sending
	/// across).
	pub fn snapshot(&mut self, batch: &Batch<'_>) -> Result<(), Error> {
		let header = batch.get_block_header(&self.head.last_block_h)?;
		self.output_pmmr.snapshot(&header).map_err(Error::Other)?;
		self.rproof_pmmr.snapshot(&header).map_err(Error::Other)?;
		Ok(())
	}

	/// Rewinds the MMRs to the provided block, rewinding to the last output pos
	/// and last kernel pos of that block. If `updated_bitmap` is supplied, the
	/// bitmap accumulator will be replaced with its contents
	pub fn rewind(&mut self, header: &BlockHeader, batch: &Batch<'_>) -> Result<(), Error> {
		debug!(
			"Rewind extension to {} at {} from {} at {}",
			header.hash(),
			header.height,
			self.head.hash(),
			self.head.height
		);

		// We need to build bitmaps of added and removed output positions
		// so we can correctly rewind all operations applied to the output MMR
		// after the position we are rewinding to (these operations will be
		// undone during rewind).
		// Rewound output pos will be removed from the MMR.
		// Rewound input (spent) pos will be added back to the MMR.
		let head_header = batch.get_block_header(&self.head.hash())?;

		if head_header.height <= header.height {
			// Nothing to rewind but we do want to truncate the MMRs at header for consistency.
			self.rewind_mmrs_to_pos(header.output_mmr_size, header.kernel_mmr_size, &[])?;
			self.apply_to_bitmap_accumulator(&[header.output_mmr_size])?;
		} else {
			let mut affected_pos = vec![];
			let mut current = head_header;
			while header.height < current.height {
				let block = batch.get_block(&current.hash())?;
				let mut affected_pos_single_block = self.rewind_single_block(&block, batch)?;
				affected_pos.append(&mut affected_pos_single_block);
				current = batch.get_previous_header(&current)?;
			}
			// Now apply a single aggregate "affected_pos" to our bitmap accumulator.
			self.apply_to_bitmap_accumulator(&affected_pos)?;
		}

		// Update our head to reflect the header we rewound to.
		self.head = Tip::from_header(header);

		Ok(())
	}

	// Rewind the MMRs and the output_pos index.
	// Returns a vec of "affected_pos" so we can apply the necessary updates to the bitmap
	// accumulator in a single pass for all rewound blocks.
	fn rewind_single_block(&mut self, block: &Block, batch: &Batch<'_>) -> Result<Vec<u64>, Error> {
		let header = &block.header;
		let prev_header = batch.get_previous_header(&header)?;

		// The spent index allows us to conveniently "unspend" everything in a block.
		let spent = batch.get_spent_index(&header.hash());

		let spent_pos: Vec<_> = if let Ok(ref spent) = spent {
			spent.iter().map(|x| x.pos).collect()
		} else {
			warn!(
				"rewind_single_block: fallback to legacy input bitmap for block {} at {}",
				header.hash(),
				header.height
			);
			let bitmap = batch.get_block_input_bitmap(&header.hash())?;
			bitmap.iter().map(|x| x.into()).collect()
		};

		if header.height == 0 {
			self.rewind_mmrs_to_pos(0, 0, &spent_pos)?;
		} else {
			let prev = batch.get_previous_header(header)?;
			self.rewind_mmrs_to_pos(prev.output_mmr_size, prev.kernel_mmr_size, &spent_pos)?;
		}

		// Update our BitmapAccumulator based on affected outputs.
		// We want to "unspend" every rewound spent output.
		// Treat size as an affected output to ensure we rebuild far enough back.
		let mut affected_pos = spent_pos;
		affected_pos.push(self.output_pmmr.size);

		// Remove any entries from the output_pos created by the block being rewound.
		let mut missing_count = 0;
		for out in block.outputs() {
			if batch.delete_output_pos_height(&out.commitment()).is_err() {
				missing_count += 1;
			}
		}
		if missing_count > 0 {
			warn!(
				"rewind_single_block: {} output_pos entries missing for: {} at {}",
				missing_count,
				header.hash(),
				header.height,
			);
		}

		// If NRD feature flag is enabled rewind the kernel_pos index
		// for any NRD kernels in the block being rewound.
		if global::is_nrd_enabled() {
			let kernel_index = store::nrd_recent_kernel_index();
			for kernel in block.kernels() {
				if let KernelFeatures::NoRecentDuplicate { .. } = kernel.features {
					kernel_index.rewind(batch, kernel.excess(), prev_header.kernel_mmr_size)?;
				}
			}
		}

		// Update output_pos based on "unspending" all spent pos from this block.
		// This is necessary to ensure the output_pos index correctly reflects a
		// reused output commitment. For example an output at pos 1, spent, reused at pos 2.
		// The output_pos index should be updated to reflect the old pos 1 when unspent.
		if let Ok(spent) = spent {
			for pos1 in spent {
				if let Some(out) = self.output_pmmr.get_data(pos1.pos - 1) {
					batch.save_output_pos_height(&out.commitment(), pos1)?;
				}
			}
		}

		Ok(affected_pos)
	}

	/// Rewinds the MMRs to the provided positions, given the output and
	/// kernel pos we want to rewind to.
	fn rewind_mmrs_to_pos(
		&mut self,
		output_pos: u64,
		kernel_pos: u64,
		spent_pos: &[u64],
	) -> Result<(), Error> {
		let bitmap: Bitmap = spent_pos.iter().map(|x| *x as u32).collect();
		self.output_pmmr
			.rewind(output_pos, &bitmap)
			.map_err(&Error::TxHashSetErr)?;
		self.rproof_pmmr
			.rewind(output_pos, &bitmap)
			.map_err(&Error::TxHashSetErr)?;
		self.kernel_pmmr
			.rewind(kernel_pos, &Bitmap::create())
			.map_err(&Error::TxHashSetErr)?;
		Ok(())
	}

	/// Current root hashes and sums (if applicable) for the Output, range proof
	/// and kernel MMRs.
	pub fn roots(&self) -> Result<TxHashSetRoots, Error> {
		Ok(TxHashSetRoots {
			output_roots: OutputRoots {
				pmmr_root: self.output_pmmr.root().map_err(|_| Error::InvalidRoot)?,
				bitmap_root: self.bitmap_accumulator.root(),
			},
			rproof_root: self.rproof_pmmr.root().map_err(|_| Error::InvalidRoot)?,
			kernel_root: self.kernel_pmmr.root().map_err(|_| Error::InvalidRoot)?,
		})
	}

	/// Validate the MMR (output, rangeproof, kernel) roots against the latest header.
	pub fn validate_roots(&self, header: &BlockHeader) -> Result<(), Error> {
		if header.height == 0 {
			return Ok(());
		}
		self.roots()?.validate(header)
	}

	/// Validate the header, output and kernel MMR sizes against the block header.
	pub fn validate_sizes(&self, header: &BlockHeader) -> Result<(), Error> {
		if header.height == 0 {
			return Ok(());
		}
		if (
			header.output_mmr_size,
			header.output_mmr_size,
			header.kernel_mmr_size,
		) != self.sizes()
		{
			Err(Error::InvalidMMRSize)
		} else {
			Ok(())
		}
	}

	fn validate_mmrs(&self) -> Result<(), Error> {
		let now = Instant::now();

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

		debug!(
			"txhashset: validated the output {}, rproof {}, kernel {} mmrs, took {}s",
			self.output_pmmr.unpruned_size(),
			self.rproof_pmmr.unpruned_size(),
			self.kernel_pmmr.unpruned_size(),
			now.elapsed().as_secs(),
		);

		Ok(())
	}

	/// Validate full kernel sums against the provided header and unspent output bitmap
	/// (for overage and kernel_offset).
	/// This is an expensive operation as we need to retrieve all the UTXOs and kernels
	/// from the respective MMRs.
	/// For a significantly faster way of validating full kernel sums see BlockSums.
	pub fn validate_kernel_sums(
		&self,
		genesis: &BlockHeader,
		header: &BlockHeader,
	) -> Result<(Commitment, Commitment), Error> {
		let now = Instant::now();

		let (utxo_sum, kernel_sum) = self.verify_kernel_sums(
			header.total_overage(genesis.kernel_mmr_size > 0),
			header.total_kernel_offset(),
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
		genesis: &BlockHeader,
		fast_validation: bool,
		status: &dyn TxHashsetWriteStatus,
		output_start_pos: Option<u64>,
		_kernel_start_pos: Option<u64>,
		header: &BlockHeader,
		stop_state: Option<Arc<StopState>>,
	) -> Result<(Commitment, Commitment), Error> {
		self.validate_mmrs()?;
		self.validate_roots(header)?;
		self.validate_sizes(header)?;

		if self.head.height == 0 {
			let zero_commit = secp_static::commit_to_zero_value();
			return Ok((zero_commit, zero_commit));
		}

		// The real magicking happens here. Sum of kernel excesses should equal
		// sum of unspent outputs minus total supply.
		let (output_sum, kernel_sum) = self.validate_kernel_sums(genesis, header)?;

		// These are expensive verification step (skipped for "fast validation").
		if !fast_validation {
			// Verify the rangeproof associated with each unspent output.
			self.verify_rangeproofs(
				Some(status),
				output_start_pos,
				None,
				false,
				stop_state.clone(),
			)?;
			if let Some(ref s) = stop_state {
				if s.is_stopped() {
					return Err(Error::Stopped.into());
				}
			}

			// Verify all the kernel signatures.
			self.verify_kernel_signatures(status, stop_state.clone())?;
			if let Some(ref s) = stop_state {
				if s.is_stopped() {
					return Err(Error::Stopped.into());
				}
			}
		}

		Ok((output_sum, kernel_sum))
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

	/// Dumps the state of the 3 MMRs to stdout for debugging. Short
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

	/// Sizes of each of the MMRs
	pub fn sizes(&self) -> (u64, u64, u64) {
		(
			self.output_pmmr.unpruned_size(),
			self.rproof_pmmr.unpruned_size(),
			self.kernel_pmmr.unpruned_size(),
		)
	}

	fn verify_kernel_signatures(
		&self,
		status: &dyn TxHashsetWriteStatus,
		stop_state: Option<Arc<StopState>>,
	) -> Result<(), Error> {
		let now = Instant::now();
		const KERNEL_BATCH_SIZE: usize = 5_000;

		let mut kern_count = 0;
		let total_kernels = pmmr::n_leaves(self.kernel_pmmr.unpruned_size());
		let mut tx_kernels: Vec<TxKernel> = Vec::with_capacity(KERNEL_BATCH_SIZE);
		for n in 0..self.kernel_pmmr.unpruned_size() {
			if pmmr::is_leaf(n) {
				let kernel = self
					.kernel_pmmr
					.get_data(n)
					.ok_or_else(|| Error::TxKernelNotFound)?;
				tx_kernels.push(kernel);
			}

			if tx_kernels.len() >= KERNEL_BATCH_SIZE || n + 1 >= self.kernel_pmmr.unpruned_size() {
				TxKernel::batch_sig_verify(&tx_kernels)?;
				kern_count += tx_kernels.len() as u64;
				tx_kernels.clear();
				status.on_validation_kernels(kern_count, total_kernels);
				if let Some(ref s) = stop_state {
					if s.is_stopped() {
						return Ok(());
					}
				}
				debug!(
					"txhashset: verify_kernel_signatures: verified {} signatures",
					kern_count,
				);
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

	fn verify_rangeproofs(
		&self,
		status: Option<&dyn TxHashsetWriteStatus>,
		start_pos: Option<u64>,
		batch_size: Option<usize>,
		single_iter: bool,
		stop_state: Option<Arc<StopState>>,
	) -> Result<u64, Error> {
		let now = Instant::now();

		let batch_size = batch_size.unwrap_or(1_000);

		let mut commits: Vec<Commitment> = Vec::with_capacity(batch_size);
		let mut proofs: Vec<RangeProof> = Vec::with_capacity(batch_size);

		let mut proof_count = 0;
		if let Some(s) = start_pos {
			if let Some(i) = pmmr::pmmr_leaf_to_insertion_index(s) {
				proof_count = self.output_pmmr.n_unpruned_leaves_to_index(i) as usize;
			}
		}

		let total_rproofs = self.output_pmmr.n_unpruned_leaves();

		for pos0 in self.output_pmmr.leaf_pos_iter() {
			if let Some(p) = start_pos {
				if pos0 < p {
					continue;
				}
			}
			let output = self.output_pmmr.get_data(pos0);
			let proof = self.rproof_pmmr.get_data(pos0);

			// Output and corresponding rangeproof *must* exist.
			// It is invalid for either to be missing and we fail immediately in this case.
			match (output, proof) {
				(None, _) => return Err(Error::OutputNotFound),
				(_, None) => return Err(Error::RangeproofNotFound),
				(Some(output), Some(proof)) => {
					commits.push(output.commit);
					proofs.push(proof);
				}
			}

			proof_count += 1;

			if proofs.len() >= batch_size {
				Output::batch_verify_proofs(&commits, &proofs)?;
				commits.clear();
				proofs.clear();
				debug!(
					"txhashset: verify_rangeproofs: verified {} rangeproofs",
					proof_count,
				);
				if let Some(s) = status {
					s.on_validation_rproofs(proof_count as u64, total_rproofs);
				}
				if let Some(ref s) = stop_state {
					if s.is_stopped() {
						return Ok(pos0);
					}
				}
				if single_iter {
					return Ok(pos0);
				}
			}
		}

		// remaining part which not full of batch_size range proofs
		if !proofs.is_empty() {
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
		Ok(0)
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
		debug!(
			"zip_read: {} at {}: reusing existing zip file: {:?}",
			header.hash(),
			header.height,
			zip_path
		);
		return Ok(zip);
	} else {
		// clean up old zips.
		// Theoretically, we only need clean-up those zip files older than STATE_SYNC_THRESHOLD.
		// But practically, these zip files are not small ones, we just keep the zips in last 24 hours
		let data_dir = Path::new(&root_dir);
		let pattern = format!("{}_", TXHASHSET_ZIP);
		if let Ok(n) = clean_files_by_prefix(data_dir, &pattern, 24 * 60 * 60) {
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

	debug!(
		"zip_read: {} at {}: created zip file: {:?}",
		header.hash(),
		header.height,
		zip_path
	);

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
	let txhashset_path = root_dir.join(TXHASHSET_SUBDIR);
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
	if let Err(e) = fs::rename(from.join(TXHASHSET_SUBDIR), to.join(TXHASHSET_SUBDIR)) {
		error!("hashset_replace fail on {}. err: {}", TXHASHSET_SUBDIR, e);
		Err(Error::TxHashSetErr("txhashset replacing fail".to_string()))
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

/// Given a block header to rewind to and the block header at the
/// head of the current chain state, we need to calculate the positions
/// of all inputs (spent outputs) we need to "undo" during a rewind.
/// We do this by leveraging the "block_input_bitmap" cache and OR'ing
/// the set of bitmaps together for the set of blocks being rewound.
fn input_pos_to_rewind(
	block_header: &BlockHeader,
	head_header: &BlockHeader,
	batch: &Batch<'_>,
) -> Result<Bitmap, Error> {
	let mut bitmap = Bitmap::create();
	let mut current = head_header.clone();
	while current.height > block_header.height {
		if let Ok(block_bitmap) = batch.get_block_input_bitmap(&current.hash()) {
			bitmap.or_inplace(&block_bitmap);
		}
		current = batch.get_previous_header(&current)?;
	}
	Ok(bitmap)
}

/// If NRD enabled then enforce NRD relative height rules.
fn apply_kernel_rules(kernel: &TxKernel, pos: CommitPos, batch: &Batch<'_>) -> Result<(), Error> {
	if !global::is_nrd_enabled() {
		return Ok(());
	}
	match kernel.features {
		KernelFeatures::NoRecentDuplicate {
			relative_height, ..
		} => {
			let kernel_index = store::nrd_recent_kernel_index();
			debug!("checking NRD index: {:?}", kernel.excess());
			if let Some(prev) = kernel_index.peek_pos(batch, kernel.excess())? {
				let diff = pos.height.saturating_sub(prev.height);
				debug!(
					"NRD check: {}, {:?}, {:?}",
					pos.height, prev, relative_height
				);
				if diff < relative_height.into() {
					return Err(Error::NRDRelativeHeight);
				}
			}
			debug!(
				"pushing entry to NRD index: {:?}: {:?}",
				kernel.excess(),
				pos,
			);
			kernel_index.push_pos(batch, kernel.excess(), pos)?;
		}
		_ => {}
	}
	Ok(())
}
