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

//! Base types that the block chain pipeline requires.

use chrono::prelude::{DateTime, Utc};
use chrono::Duration;
use grin_core::core::{OutputIdentifier, Segment, SegmentType, TxKernel};
use grin_util::secp::pedersen::RangeProof;
use std::net::SocketAddr;
use std::sync::Weak;

use crate::core::core::hash::{Hash, Hashed, ZERO_HASH};
use crate::core::core::{pmmr, Block, BlockHeader, HeaderVersion, SegmentTypeIdentifier};
use crate::core::pow::Difficulty;
use crate::core::ser::{self, PMMRIndexHashable, Readable, Reader, Writeable, Writer};
use crate::error::Error;
use crate::txhashset::BitmapChunk;
use crate::util::{RwLock, RwLockWriteGuard};

bitflags! {
/// Options for block validation
	pub struct Options: u32 {
		/// No flags
		const NONE = 0b0000_0000;
		/// Runs without checking the Proof of Work, mostly to make testing easier.
		const SKIP_POW = 0b0000_0001;
		/// Adds block while in syncing mode.
		const SYNC = 0b0000_0010;
		/// Block validation on a block we mined ourselves
		const MINE = 0b0000_0100;
	}
}

/// Various status sync can be in, whether it's fast sync or archival.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum SyncStatus {
	/// Initial State (we do not yet know if we are/should be syncing)
	Initial,
	/// Not syncing
	NoSync,
	/// Not enough peers to do anything yet, boolean indicates whether
	/// we should wait at all or ignore and start ASAP
	AwaitingPeers(bool),
	/// Downloading block headers
	HeaderSync {
		/// current sync head
		sync_head: Tip,
		/// height of the most advanced peer
		highest_height: u64,
		/// diff of the most advanced peer
		highest_diff: Difficulty,
	},
	/// Performing PIBD reconstruction of txhashset
	/// If PIBD syncer determines there's not enough
	/// PIBD peers to continue, then move on to TxHashsetDownload state
	TxHashsetPibd {
		/// Whether the syncer has determined there's not enough
		/// data to continue via PIBD
		aborted: bool,
		/// whether we got an error anywhere (in which case restart the process)
		errored: bool,
		/// total number of leaves applied
		completed_leaves: u64,
		/// total number of leaves required by archive header
		leaves_required: u64,
		/// 'height', i.e. last 'block' for which there is complete
		/// pmmr data
		completed_to_height: u64,
		/// Total 'height' needed
		required_height: u64,
	},
	/// Downloading the various txhashsets
	TxHashsetDownload(TxHashsetDownloadStats),
	/// Setting up before validation
	TxHashsetSetup {
		/// number of 'headers' for which kernels have been checked
		headers: Option<u64>,
		/// headers total
		headers_total: Option<u64>,
		/// kernel position portion
		kernel_pos: Option<u64>,
		/// total kernel position
		kernel_pos_total: Option<u64>,
	},
	/// Validating the kernels
	TxHashsetKernelsValidation {
		/// kernels validated
		kernels: u64,
		/// kernels in total
		kernels_total: u64,
	},
	/// Validating the range proofs
	TxHashsetRangeProofsValidation {
		/// range proofs validated
		rproofs: u64,
		/// range proofs in total
		rproofs_total: u64,
	},
	/// Finalizing the new state
	TxHashsetSave,
	/// State sync finalized
	TxHashsetDone,
	/// Downloading blocks
	BodySync {
		/// current node height
		current_height: u64,
		/// height of the most advanced peer
		highest_height: u64,
	},
	/// Shutdown
	Shutdown,
}

/// Stats for TxHashsetDownload stage
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
pub struct TxHashsetDownloadStats {
	/// when download started
	pub start_time: DateTime<Utc>,
	/// time of the previous update
	pub prev_update_time: DateTime<Utc>,
	/// time of the latest update
	pub update_time: DateTime<Utc>,
	/// size of the previous chunk
	pub prev_downloaded_size: u64,
	/// size of the the latest chunk
	pub downloaded_size: u64,
	/// downloaded since the start
	pub total_size: u64,
}

impl Default for TxHashsetDownloadStats {
	fn default() -> Self {
		TxHashsetDownloadStats {
			start_time: Utc::now(),
			update_time: Utc::now(),
			prev_update_time: Utc::now(),
			prev_downloaded_size: 0,
			downloaded_size: 0,
			total_size: 0,
		}
	}
}

/// PIBD segment type to process.
#[derive(Clone)]
pub enum PIBDSegment {
	/// Bitmap (block hash, output root, segment).
	Bitmap(Hash, Hash, Segment<BitmapChunk>),
	/// Output (block hash, bitmap root, segment).
	Output(Hash, Hash, Segment<OutputIdentifier>),
	/// RangeProof (block hash, segment).
	RangeProof(Hash, Segment<RangeProof>),
	/// Kernel (block hash, segment).
	Kernel(Hash, Segment<TxKernel>),
}

impl PIBDSegment {
	/// Get PIBD segment identifier.
	pub fn segment_id(&self) -> SegmentTypeIdentifier {
		match self {
			PIBDSegment::Bitmap(_, _, segment) => SegmentTypeIdentifier {
				segment_type: SegmentType::Bitmap,
				identifier: segment.identifier().clone(),
			},
			PIBDSegment::Output(_, _, segment) => SegmentTypeIdentifier {
				segment_type: SegmentType::Output,
				identifier: segment.identifier().clone(),
			},
			PIBDSegment::RangeProof(_, segment) => SegmentTypeIdentifier {
				segment_type: SegmentType::RangeProof,
				identifier: segment.identifier().clone(),
			},
			PIBDSegment::Kernel(_, segment) => SegmentTypeIdentifier {
				segment_type: SegmentType::Kernel,
				identifier: segment.identifier().clone(),
			},
		}
	}
}

/// Received PIBD segment to process from the peer.
#[derive(Clone)]
pub struct QueuedPIBDSegment {
	/// Peer from where segment was received.
	pub peer_addr: SocketAddr,
	/// Archive hash.
	pub archive_hash: Hash,
	/// Segment.
	pub segment: PIBDSegment,
}

/// Container for entry in requested PIBD segments
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PIBDSegmentContainer {
	/// Segment+Type Identifier
	pub identifier: SegmentTypeIdentifier,
	/// Time at which this request was made
	pub request_time: DateTime<Utc>,
	/// Peer that most recently received this request
	pub last_peer: Option<SocketAddr>,
	/// Archive header for this request
	pub archive_hash: Hash,
}

impl PIBDSegmentContainer {
	/// Return container with timestamp
	pub fn new(
		identifier: SegmentTypeIdentifier,
		peer_addr: Option<SocketAddr>,
		archive_hash: Hash,
	) -> Self {
		Self {
			identifier,
			request_time: Utc::now(),
			last_peer: peer_addr,
			archive_hash,
		}
	}
}

/// Recently rejected PIBD segment from a specific peer.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RejectedPIBDSegment {
	/// Segment+Type Identifier
	pub identifier: SegmentTypeIdentifier,
	/// Peer that provided invalid data for this segment
	pub peer_addr: SocketAddr,
	/// Time at which this segment was rejected
	pub reject_time: DateTime<Utc>,
}

/// Recent PIBD rejections for a peer.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RejectedPIBDPeer {
	/// Peer address
	pub peer_addr: SocketAddr,
	/// Last rejection time
	pub reject_time: DateTime<Utc>,
	/// Recent rejection count
	pub reject_count: u32,
}

const MAX_REJECTED_PIBD_SEGMENTS: usize = 1024;
const MAX_REJECTED_PIBD_PEERS: usize = 1024;
const REJECTED_PIBD_PEER_THRESHOLD: u32 = 3;

/// Current sync state. Encapsulates the current SyncStatus.
pub struct SyncState {
	current: RwLock<SyncStatus>,
	sync_error: RwLock<Option<Error>>,
	/// Something has to keep track of segments that have been
	/// requested from other peers. TODO consider: This may not
	/// be the best place to put code that's concerned with peers
	/// but it's currently the only place that makes the info
	/// available where it will be needed (both in the adapter
	/// and the sync loop)
	requested_pibd_segments: RwLock<Vec<PIBDSegmentContainer>>,
	rejected_pibd_segments: RwLock<Vec<RejectedPIBDSegment>>,
	rejected_pibd_peers: RwLock<Vec<RejectedPIBDPeer>>,
}

impl SyncState {
	/// Return a new SyncState initialize to NoSync
	pub fn new() -> SyncState {
		SyncState {
			current: RwLock::new(SyncStatus::Initial),
			sync_error: RwLock::new(None),
			requested_pibd_segments: RwLock::new(vec![]),
			rejected_pibd_segments: RwLock::new(vec![]),
			rejected_pibd_peers: RwLock::new(vec![]),
		}
	}

	/// Reset sync status to NoSync.
	pub fn reset(&self) {
		self.clear_sync_error();
		self.update(SyncStatus::NoSync);
	}

	/// Whether the current state matches any active syncing operation.
	/// Note: This includes our "initial" state.
	pub fn is_syncing(&self) -> bool {
		*self.current.read() != SyncStatus::NoSync
	}

	/// Current syncing status
	pub fn status(&self) -> SyncStatus {
		*self.current.read()
	}

	/// Update the syncing status
	pub fn update(&self, new_status: SyncStatus) -> bool {
		let status = self.current.write();
		self.update_with_guard(new_status, status)
	}

	fn update_with_guard(
		&self,
		new_status: SyncStatus,
		mut status: RwLockWriteGuard<SyncStatus>,
	) -> bool {
		if *status == new_status {
			return false;
		}

		debug!("sync_state: sync_status: {:?} -> {:?}", *status, new_status,);
		*status = new_status;
		true
	}

	/// Update the syncing status if predicate f is satisfied
	pub fn update_if<F>(&self, new_status: SyncStatus, f: F) -> bool
	where
		F: Fn(SyncStatus) -> bool,
	{
		let status = self.current.write();
		if f(*status) {
			self.update_with_guard(new_status, status)
		} else {
			false
		}
	}

	/// Update sync_head if state is currently HeaderSync.
	pub fn update_header_sync(&self, new_sync_head: Tip) {
		let status: &mut SyncStatus = &mut self.current.write();
		match status {
			SyncStatus::HeaderSync { sync_head, .. } => {
				*sync_head = new_sync_head;
			}
			_ => (),
		}
	}

	/// Update txhashset downloading progress
	pub fn update_txhashset_download(&self, stats: TxHashsetDownloadStats) {
		*self.current.write() = SyncStatus::TxHashsetDownload(stats);
	}

	/// Update PIBD progress
	pub fn update_pibd_progress(
		&self,
		aborted: bool,
		errored: bool,
		completed_leaves: u64,
		completed_to_height: u64,
		archive_header: &BlockHeader,
	) {
		let leaves_required = pmmr::n_leaves(archive_header.output_mmr_size) * 2
			+ pmmr::n_leaves(archive_header.kernel_mmr_size);
		*self.current.write() = SyncStatus::TxHashsetPibd {
			aborted,
			errored,
			completed_leaves,
			leaves_required,
			completed_to_height,
			required_height: archive_header.height,
		};
	}

	/// Update lightweight PIBD leaf progress for TUI/API display.
	pub fn update_pibd_leaf_progress(&self, completed_leaves: u64, archive_header: &BlockHeader) {
		let leaves_required = pmmr::n_leaves(archive_header.output_mmr_size) * 2
			+ pmmr::n_leaves(archive_header.kernel_mmr_size);
		let status: &mut SyncStatus = &mut self.current.write();
		match status {
			SyncStatus::TxHashsetPibd {
				completed_leaves: current_completed_leaves,
				leaves_required: current_leaves_required,
				required_height,
				..
			} => {
				*current_completed_leaves = completed_leaves;
				*current_leaves_required = leaves_required;
				*required_height = archive_header.height;
			}
			_ => {
				*status = SyncStatus::TxHashsetPibd {
					aborted: false,
					errored: false,
					completed_leaves,
					leaves_required,
					completed_to_height: 0,
					required_height: archive_header.height,
				};
			}
		}
	}

	/// Process PIBD segment.
	pub fn process_queued_pibd_segment(
		&self,
		chain: &Weak<crate::Chain>,
		queued_segment: QueuedPIBDSegment,
	) -> Result<(), Error> {
		let peer_addr = queued_segment.peer_addr;
		let archive_hash = queued_segment.archive_hash;
		let chain = chain
			.upgrade()
			.ok_or_else(|| Error::Other("chain not available".to_owned()))?;
		let archive_header = chain.txhashset_archive_header_header_only()?;
		let segment_id = queued_segment.segment.segment_id();
		if archive_hash != archive_header.hash() {
			debug!(
				"dropping stale PIBD segment {:?} from {} for archive header {} (current {})",
				segment_id,
				peer_addr,
				archive_hash,
				archive_header.hash(),
			);
			self.remove_pibd_segment_for_archive(&segment_id, peer_addr, archive_hash);
			return Ok(());
		}
		let desegmenter = chain.desegmenter(&archive_header)?;
		let mut desegmenter = desegmenter.write();
		let res = if let Some(d) = desegmenter.as_mut() {
			match queued_segment.segment {
				PIBDSegment::Bitmap(block_hash, output_root, segment) => {
					debug!(
						"Received bitmap segment {} for block_hash: {}, output_root: {}",
						segment.identifier().idx,
						block_hash,
						output_root
					);
					d.add_bitmap_segment(segment, output_root)
				}
				PIBDSegment::Output(block_hash, bitmap_root, segment) => {
					debug!(
						"Received output segment {} for block_hash: {}, bitmap_root: {:?}",
						segment.identifier().idx,
						block_hash,
						bitmap_root,
					);
					d.add_output_segment(segment)
				}
				PIBDSegment::RangeProof(block_hash, segment) => {
					debug!(
						"Received proof segment {} for block_hash: {}",
						segment.identifier().idx,
						block_hash,
					);
					d.add_rangeproof_segment(segment)
				}
				PIBDSegment::Kernel(block_hash, segment) => {
					debug!(
						"Received kernel segment {} for block_hash: {}",
						segment.identifier().idx,
						block_hash,
					);
					d.add_kernel_segment(segment)
				}
			}
		} else {
			Ok(())
		};
		if res.is_err() {
			warn!(
				"PIBD segment {:?} from peer {} failed validation",
				segment_id, peer_addr
			);
			self.reject_pibd_segment_from(&segment_id, peer_addr);
		} else {
			self.remove_pibd_segment_for_archive(&segment_id, peer_addr, archive_hash);
		}
		res
	}

	/// Update PIBD segment list
	pub fn add_pibd_segment(
		&self,
		id: &SegmentTypeIdentifier,
		peer_addr: SocketAddr,
		archive_hash: Hash,
	) {
		debug!("sync_state: tracking PIBD request for {:?}", id);
		let mut requested_segments = self.requested_pibd_segments.write();
		if let Some(existing) = requested_segments.iter_mut().find(|i| &i.identifier == id) {
			existing.request_time = Utc::now();
			existing.last_peer = Some(peer_addr);
			existing.archive_hash = archive_hash;
		} else {
			requested_segments.push(PIBDSegmentContainer::new(
				id.clone(),
				Some(peer_addr),
				archive_hash,
			));
		}
	}

	/// Remove segment from list
	pub fn remove_pibd_segment(&self, id: &SegmentTypeIdentifier) {
		debug!("sync_state: removing PIBD request tracking for {:?}", id);
		self.requested_pibd_segments
			.write()
			.retain(|i| &i.identifier != id);
	}

	/// Remove segment from list only if it is still pending for the given peer.
	pub fn remove_pibd_segment_from(&self, id: &SegmentTypeIdentifier, peer_addr: SocketAddr) {
		trace!(
			"sync_state: removing PIBD request tracking for {:?} from {}",
			id,
			peer_addr,
		);
		self.requested_pibd_segments
			.write()
			.retain(|i| &i.identifier != id || i.last_peer != Some(peer_addr));
	}

	/// Remove segment from list only if it is still pending for the given peer and archive.
	pub fn remove_pibd_segment_for_archive(
		&self,
		id: &SegmentTypeIdentifier,
		peer_addr: SocketAddr,
		archive_hash: Hash,
	) {
		trace!(
			"sync_state: removing PIBD request tracking for {:?} from {} and archive {}",
			id,
			peer_addr,
			archive_hash
		);
		self.requested_pibd_segments.write().retain(|i| {
			&i.identifier != id || i.last_peer != Some(peer_addr) || i.archive_hash != archive_hash
		});
	}

	/// Return the archive hash for a pending request from the given peer.
	pub fn get_pibd_segment_archive_hash(
		&self,
		id: &SegmentTypeIdentifier,
		peer_addr: SocketAddr,
	) -> Option<Hash> {
		self.requested_pibd_segments
			.read()
			.iter()
			.find(|i| &i.identifier == id && i.last_peer == Some(peer_addr))
			.map(|i| i.archive_hash)
	}

	/// Remove segments with request timestamps less than cutoff time
	pub fn remove_stale_pibd_requests(
		&self,
		timeout_seconds: i64,
	) -> Vec<(SegmentTypeIdentifier, Option<SocketAddr>)> {
		let cutoff_time = Utc::now() - Duration::seconds(timeout_seconds);
		let mut removed_segments = vec![];
		self.requested_pibd_segments.write().retain(|i| {
			if i.request_time <= cutoff_time {
				debug!("Removing + retrying PIBD request after timeout: {:?}", i);
				removed_segments.push((i.identifier.clone(), i.last_peer));
				false
			} else {
				true
			}
		});
		removed_segments
	}

	/// Check whether segment is in request list
	pub fn contains_pibd_segment(&self, id: &SegmentTypeIdentifier) -> bool {
		self.requested_pibd_segments
			.read()
			.iter()
			.any(|i| &i.identifier == id)
	}

	/// Mark a requested PIBD segment as rejected for this peer.
	pub fn reject_pibd_segment_from(&self, id: &SegmentTypeIdentifier, peer_addr: SocketAddr) {
		self.remove_pibd_segment_from(id, peer_addr);
		let mut rejected = self.rejected_pibd_segments.write();
		rejected.retain(|i| &i.identifier != id || i.peer_addr != peer_addr);
		rejected.push(RejectedPIBDSegment {
			identifier: id.clone(),
			peer_addr,
			reject_time: Utc::now(),
		});
		if rejected.len() > MAX_REJECTED_PIBD_SEGMENTS {
			rejected.remove(0);
		}
		drop(rejected);

		let mut rejected_peers = self.rejected_pibd_peers.write();
		if let Some(existing) = rejected_peers.iter_mut().find(|i| i.peer_addr == peer_addr) {
			existing.reject_time = Utc::now();
			existing.reject_count = existing.reject_count.saturating_add(1);
		} else {
			rejected_peers.push(RejectedPIBDPeer {
				peer_addr,
				reject_time: Utc::now(),
				reject_count: 1,
			});
		}
		if rejected_peers.len() > MAX_REJECTED_PIBD_PEERS {
			rejected_peers.remove(0);
		}
	}

	/// Check whether this peer recently provided invalid PIBD data.
	pub fn rejected_pibd_segment_from_peer(
		&self,
		id: &SegmentTypeIdentifier,
		peer_addr: SocketAddr,
	) -> bool {
		let cutoff_time =
			Utc::now() - Duration::seconds(crate::pibd_params::REJECTED_SEGMENT_RETRY_SECS);
		let rejected = self.rejected_pibd_segments.read();
		if rejected
			.iter()
			.any(|i| &i.identifier == id && i.peer_addr == peer_addr && i.reject_time > cutoff_time)
		{
			return true;
		}

		self.rejected_pibd_peers.read().iter().any(|i| {
			i.peer_addr == peer_addr
				&& i.reject_time > cutoff_time
				&& i.reject_count >= REJECTED_PIBD_PEER_THRESHOLD
		})
	}

	/// Prune expired PIBD rejection entries.
	pub fn prune_rejected_pibd_segments(&self, reject_seconds: i64) {
		let cutoff_time = Utc::now() - Duration::seconds(reject_seconds);
		self.rejected_pibd_segments
			.write()
			.retain(|i| i.reject_time > cutoff_time);
		self.rejected_pibd_peers
			.write()
			.retain(|i| i.reject_time > cutoff_time);
	}

	/// Number of currently pending PIBD segment requests
	pub fn pending_pibd_segment_count(&self) -> usize {
		self.requested_pibd_segments.read().len()
	}

	/// Communicate sync error
	pub fn set_sync_error(&self, error: Error) {
		*self.sync_error.write() = Some(error);
	}

	/// Get sync error
	pub fn sync_error(&self) -> Option<String> {
		self.sync_error.read().as_ref().map(|e| e.to_string())
	}

	/// Clear sync error
	pub fn clear_sync_error(&self) {
		*self.sync_error.write() = None;
	}
}

impl TxHashsetWriteStatus for SyncState {
	fn on_setup(
		&self,
		headers: Option<u64>,
		headers_total: Option<u64>,
		kernel_pos: Option<u64>,
		kernel_pos_total: Option<u64>,
	) {
		self.update(SyncStatus::TxHashsetSetup {
			headers,
			headers_total,
			kernel_pos,
			kernel_pos_total,
		});
	}

	fn on_validation_kernels(&self, kernels: u64, kernels_total: u64) {
		self.update(SyncStatus::TxHashsetKernelsValidation {
			kernels,
			kernels_total,
		});
	}

	fn on_validation_rproofs(&self, rproofs: u64, rproofs_total: u64) {
		self.update(SyncStatus::TxHashsetRangeProofsValidation {
			rproofs,
			rproofs_total,
		});
	}

	fn on_save(&self) {
		self.update(SyncStatus::TxHashsetSave);
	}

	fn on_done(&self) {
		self.update(SyncStatus::TxHashsetDone);
	}
}

/// A helper for the various txhashset MMR roots.
#[derive(Debug)]
pub struct TxHashSetRoots {
	/// Output roots
	pub output_roots: OutputRoots,
	/// Range Proof root
	pub rproof_root: Hash,
	/// Kernel root
	pub kernel_root: Hash,
}

impl TxHashSetRoots {
	/// Accessor for the output PMMR root (rules here are block height dependent).
	/// We assume the header version is consistent with the block height, validated
	/// as part of pipe::validate_header().
	pub fn output_root(&self, header: &BlockHeader) -> Hash {
		self.output_roots.root(header)
	}

	/// Validate roots against the provided block header.
	pub fn validate(&self, header: &BlockHeader) -> Result<(), Error> {
		debug!(
			"validate roots: {} at {}, {} vs. {} (original: {}, merged: {})",
			header.hash(),
			header.height,
			header.output_root,
			self.output_root(header),
			self.output_roots.pmmr_root,
			self.output_roots.merged_root(header),
		);

		if header.output_root != self.output_root(header)
			|| header.range_proof_root != self.rproof_root
			|| header.kernel_root != self.kernel_root
		{
			Err(Error::InvalidRoot)
		} else {
			Ok(())
		}
	}
}

/// A helper for the various output roots.
#[derive(Debug)]
pub struct OutputRoots {
	/// The output PMMR root
	pub pmmr_root: Hash,
	/// The bitmap accumulator root
	pub bitmap_root: Hash,
}

impl OutputRoots {
	/// The root of our output PMMR. The rules here are block height specific.
	/// We use the merged root here for header version 3 and later.
	/// We assume the header version is consistent with the block height, validated
	/// as part of pipe::validate_header().
	pub fn root(&self, header: &BlockHeader) -> Hash {
		if header.version < HeaderVersion(3) {
			self.output_root()
		} else {
			self.merged_root(header)
		}
	}

	/// The root of the underlying output PMMR.
	fn output_root(&self) -> Hash {
		self.pmmr_root
	}

	/// Hash the root of the output PMMR and the root of the bitmap accumulator
	/// together with the size of the output PMMR (for consistency with existing PMMR impl).
	/// H(pmmr_size | pmmr_root | bitmap_root)
	fn merged_root(&self, header: &BlockHeader) -> Hash {
		(self.pmmr_root, self.bitmap_root).hash_with_index(header.output_mmr_size)
	}
}

/// Minimal struct representing a known MMR position and associated block height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommitPos {
	/// MMR position
	pub pos: u64,
	/// Block height
	pub height: u64,
}

impl Readable for CommitPos {
	fn read<R: Reader>(reader: &mut R) -> Result<CommitPos, ser::Error> {
		let pos = reader.read_u64()?;
		let height = reader.read_u64()?;
		Ok(CommitPos { pos, height })
	}
}

impl Writeable for CommitPos {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.pos)?;
		writer.write_u64(self.height)?;
		Ok(())
	}
}

/// The tip of a fork. A handle to the fork ancestry from its leaf in the
/// blockchain tree. References the max height and the latest and previous
/// blocks
/// for convenience and the total difficulty.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct Tip {
	/// Height of the tip (max height of the fork)
	pub height: u64,
	/// Last block pushed to the fork
	pub last_block_h: Hash,
	/// Previous block
	pub prev_block_h: Hash,
	/// Total difficulty accumulated on that fork
	pub total_difficulty: Difficulty,
}

impl Tip {
	/// Creates a new tip based on provided header.
	pub fn from_header(header: &BlockHeader) -> Tip {
		header.into()
	}
}

impl From<BlockHeader> for Tip {
	fn from(header: BlockHeader) -> Self {
		Self::from(&header)
	}
}

impl From<&BlockHeader> for Tip {
	fn from(header: &BlockHeader) -> Self {
		Tip {
			height: header.height,
			last_block_h: header.hash(),
			prev_block_h: header.prev_hash,
			total_difficulty: header.total_difficulty(),
		}
	}
}

impl Hashed for Tip {
	/// The hash of the underlying block.
	fn hash(&self) -> Hash {
		self.last_block_h
	}
}

impl Default for Tip {
	fn default() -> Self {
		Tip {
			height: 0,
			last_block_h: ZERO_HASH,
			prev_block_h: ZERO_HASH,
			total_difficulty: Difficulty::min_dma(),
		}
	}
}

/// Serialization of a tip, required to save to datastore.
impl ser::Writeable for Tip {
	fn write<W: ser::Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.height)?;
		writer.write_fixed_bytes(&self.last_block_h)?;
		writer.write_fixed_bytes(&self.prev_block_h)?;
		self.total_difficulty.write(writer)
	}
}

impl ser::Readable for Tip {
	fn read<R: ser::Reader>(reader: &mut R) -> Result<Tip, ser::Error> {
		let height = reader.read_u64()?;
		let last = Hash::read(reader)?;
		let prev = Hash::read(reader)?;
		let diff = Difficulty::read(reader)?;
		Ok(Tip {
			height: height,
			last_block_h: last,
			prev_block_h: prev,
			total_difficulty: diff,
		})
	}
}

/// Bridge between the chain pipeline and the rest of the system. Handles
/// downstream processing of valid blocks by the rest of the system, most
/// importantly the broadcasting of blocks to our peers.
pub trait ChainAdapter {
	/// The blockchain pipeline has accepted this block as valid and added
	/// it to our chain.
	fn block_accepted(&self, block: &Block, status: BlockStatus, opts: Options);
}

/// Inform the caller of the current status of a txhashset write operation,
/// as it can take quite a while to process. Each function is called in the
/// order defined below and can be used to provide some feedback to the
/// caller. Functions taking arguments can be called repeatedly to update
/// those values as the processing progresses.
pub trait TxHashsetWriteStatus {
	/// First setup of the txhashset
	fn on_setup(
		&self,
		headers: Option<u64>,
		header_total: Option<u64>,
		kernel_pos: Option<u64>,
		kernel_pos_total: Option<u64>,
	);
	/// Starting kernel validation
	fn on_validation_kernels(&self, kernels: u64, kernel_total: u64);
	/// Starting rproof validation
	fn on_validation_rproofs(&self, rproofs: u64, rproof_total: u64);
	/// Starting to save the txhashset and related data
	fn on_save(&self);
	/// Done writing a new txhashset
	fn on_done(&self);
}

/// Do-nothing implementation of TxHashsetWriteStatus
pub struct NoStatus;

impl TxHashsetWriteStatus for NoStatus {
	fn on_setup(&self, _hs: Option<u64>, _ht: Option<u64>, _kp: Option<u64>, _kpt: Option<u64>) {}
	fn on_validation_kernels(&self, _ks: u64, _kts: u64) {}
	fn on_validation_rproofs(&self, _rs: u64, _rt: u64) {}
	fn on_save(&self) {}
	fn on_done(&self) {}
}

/// Dummy adapter used as a placeholder for real implementations
pub struct NoopAdapter {}

impl ChainAdapter for NoopAdapter {
	fn block_accepted(&self, _b: &Block, _status: BlockStatus, _opts: Options) {}
}

/// Status of an accepted block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlockStatus {
	/// Block is the "next" block, updating the chain head.
	Next {
		/// Previous block (previous chain head).
		prev: Tip,
	},
	/// Block does not update the chain head and is a fork.
	Fork {
		/// Previous block on this fork.
		prev: Tip,
		/// Current chain head.
		head: Tip,
		/// Fork point for rewind.
		fork_point: Tip,
	},
	/// Block updates the chain head via a (potentially disruptive) "reorg".
	/// Previous block was not our previous chain head.
	Reorg {
		/// Previous block on this fork.
		prev: Tip,
		/// Previous chain head.
		prev_head: Tip,
		/// Fork point for rewind.
		fork_point: Tip,
	},
}

impl BlockStatus {
	/// Is this the "next" block?
	pub fn is_next(&self) -> bool {
		match *self {
			BlockStatus::Next { .. } => true,
			_ => false,
		}
	}

	/// Is this block a "reorg"?
	pub fn is_reorg(&self) -> bool {
		match *self {
			BlockStatus::Reorg { .. } => true,
			_ => false,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::core::{SegmentIdentifier, SegmentType};
	use std::net::{IpAddr, Ipv4Addr, SocketAddr};

	#[test]
	fn rejected_pibd_segment_tracking_is_bounded() {
		let sync_state = SyncState::new();

		for idx in 0..(MAX_REJECTED_PIBD_SEGMENTS + 10) {
			let id = SegmentTypeIdentifier::new(
				SegmentType::Kernel,
				SegmentIdentifier {
					height: 9,
					idx: idx as u64,
				},
			);
			let peer_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 10_000 + idx as u16);
			sync_state.reject_pibd_segment_from(&id, peer_addr);
		}

		assert_eq!(
			sync_state.rejected_pibd_segments.read().len(),
			MAX_REJECTED_PIBD_SEGMENTS
		);
	}

	#[test]
	fn remove_pibd_segment_from_peer() {
		let sync_state = SyncState::new();
		let id = SegmentTypeIdentifier::new(
			SegmentType::Kernel,
			SegmentIdentifier { height: 9, idx: 1 },
		);
		let peer_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 10_000);
		let archive_hash = Hash::from_vec(&[1; 32]);

		sync_state.add_pibd_segment(&id, peer_addr, archive_hash);

		sync_state.remove_pibd_segment_from(&id, peer_addr);
		assert!(!sync_state.contains_pibd_segment(&id));

		sync_state.remove_pibd_segment_from(&id, peer_addr);
		assert!(!sync_state.contains_pibd_segment(&id));
	}

	#[test]
	fn get_pibd_segment_archive_hash() {
		let sync_state = SyncState::new();
		let id = SegmentTypeIdentifier::new(
			SegmentType::Kernel,
			SegmentIdentifier { height: 9, idx: 1 },
		);
		let peer_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 10_000);
		let archive_hash = Hash::from_vec(&[1; 32]);

		sync_state.add_pibd_segment(&id, peer_addr, archive_hash);

		assert_eq!(
			sync_state.get_pibd_segment_archive_hash(&id, peer_addr),
			Some(archive_hash)
		);
		assert!(sync_state.contains_pibd_segment(&id));
	}

	#[test]
	fn remove_pibd_segment_for_archive() {
		let sync_state = SyncState::new();
		let id = SegmentTypeIdentifier::new(
			SegmentType::Kernel,
			SegmentIdentifier { height: 9, idx: 1 },
		);
		let peer_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 10_000);
		let archive_hash = Hash::from_vec(&[1; 32]);
		let other_archive_hash = Hash::from_vec(&[2; 32]);

		sync_state.add_pibd_segment(&id, peer_addr, archive_hash);
		sync_state.remove_pibd_segment_for_archive(&id, peer_addr, other_archive_hash);

		assert!(sync_state.contains_pibd_segment(&id));

		sync_state.remove_pibd_segment_for_archive(&id, peer_addr, archive_hash);

		assert!(!sync_state.contains_pibd_segment(&id));
	}

	#[test]
	fn rejected_pibd_peer_threshold() {
		let sync_state = SyncState::new();
		let peer_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 10_000);

		for idx in 0..(REJECTED_PIBD_PEER_THRESHOLD - 1) {
			let id = SegmentTypeIdentifier::new(
				SegmentType::Kernel,
				SegmentIdentifier {
					height: 9,
					idx: idx as u64,
				},
			);
			sync_state.reject_pibd_segment_from(&id, peer_addr);
		}

		let rejected_id = SegmentTypeIdentifier::new(
			SegmentType::Kernel,
			SegmentIdentifier { height: 9, idx: 0 },
		);
		assert!(sync_state.rejected_pibd_segment_from_peer(&rejected_id, peer_addr));

		let other_id = SegmentTypeIdentifier::new(
			SegmentType::Kernel,
			SegmentIdentifier {
				height: 9,
				idx: (REJECTED_PIBD_PEER_THRESHOLD + 1) as u64,
			},
		);
		assert!(!sync_state.rejected_pibd_segment_from_peer(&other_id, peer_addr));

		let id = SegmentTypeIdentifier::new(
			SegmentType::Kernel,
			SegmentIdentifier {
				height: 9,
				idx: REJECTED_PIBD_PEER_THRESHOLD as u64,
			},
		);
		sync_state.reject_pibd_segment_from(&id, peer_addr);

		assert!(sync_state.rejected_pibd_segment_from_peer(&other_id, peer_addr));
	}
}
