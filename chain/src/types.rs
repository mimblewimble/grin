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

//! Base types that the block chain pipeline requires.

use chrono::prelude::{DateTime, Utc};
use std::sync::Arc;

use crate::core::core::hash::{Hash, Hashed, ZERO_HASH};
use crate::core::core::{Block, BlockHeader};
use crate::core::pow::Difficulty;
use crate::core::ser;
use crate::error::Error;
use crate::util::RwLock;

bitflags! {
/// Options for block validation
	pub struct Options: u32 {
		/// No flags
		const NONE = 0b00000000;
		/// Runs without checking the Proof of Work, mostly to make testing easier.
		const SKIP_POW = 0b00000001;
		/// Adds block while in syncing mode.
		const SYNC = 0b00000010;
		/// Block validation on a block we mined ourselves
		const MINE = 0b00000100;
	}
}

/// Various status sync can be in, whether it's fast sync or archival.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[allow(missing_docs)]
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
		current_height: u64,
		highest_height: u64,
	},
	/// Downloading the various txhashsets
	TxHashsetDownload {
		start_time: DateTime<Utc>,
		prev_update_time: DateTime<Utc>,
		update_time: DateTime<Utc>,
		prev_downloaded_size: u64,
		downloaded_size: u64,
		total_size: u64,
	},
	/// Setting up before validation
	TxHashsetSetup,
	/// Validating the full state
	TxHashsetValidation {
		kernels: u64,
		kernel_total: u64,
		rproofs: u64,
		rproof_total: u64,
	},
	/// Finalizing the new state
	TxHashsetSave,
	/// State sync finalized
	TxHashsetDone,
	/// Downloading blocks
	BodySync {
		current_height: u64,
		highest_height: u64,
	},
	Shutdown,
}

/// Current sync state. Encapsulates the current SyncStatus.
pub struct SyncState {
	current: RwLock<SyncStatus>,
	sync_error: Arc<RwLock<Option<Error>>>,
}

impl SyncState {
	/// Return a new SyncState initialize to NoSync
	pub fn new() -> SyncState {
		SyncState {
			current: RwLock::new(SyncStatus::Initial),
			sync_error: Arc::new(RwLock::new(None)),
		}
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
	pub fn update(&self, new_status: SyncStatus) {
		if self.status() == new_status {
			return;
		}

		let mut status = self.current.write();

		debug!("sync_state: sync_status: {:?} -> {:?}", *status, new_status,);

		*status = new_status;
	}

	/// Update txhashset downloading progress
	pub fn update_txhashset_download(&self, new_status: SyncStatus) -> bool {
		if let SyncStatus::TxHashsetDownload { .. } = new_status {
			let mut status = self.current.write();
			*status = new_status;
			true
		} else {
			false
		}
	}

	/// Communicate sync error
	pub fn set_sync_error(&self, error: Error) {
		*self.sync_error.write() = Some(error);
	}

	/// Get sync error
	pub fn sync_error(&self) -> Arc<RwLock<Option<Error>>> {
		Arc::clone(&self.sync_error)
	}

	/// Clear sync error
	pub fn clear_sync_error(&self) {
		*self.sync_error.write() = None;
	}
}

impl TxHashsetWriteStatus for SyncState {
	fn on_setup(&self) {
		self.update(SyncStatus::TxHashsetSetup);
	}

	fn on_validation(&self, vkernels: u64, vkernel_total: u64, vrproofs: u64, vrproof_total: u64) {
		let mut status = self.current.write();
		match *status {
			SyncStatus::TxHashsetValidation {
				kernels,
				kernel_total,
				rproofs,
				rproof_total,
			} => {
				let ks = if vkernels > 0 { vkernels } else { kernels };
				let kt = if vkernel_total > 0 {
					vkernel_total
				} else {
					kernel_total
				};
				let rps = if vrproofs > 0 { vrproofs } else { rproofs };
				let rpt = if vrproof_total > 0 {
					vrproof_total
				} else {
					rproof_total
				};
				*status = SyncStatus::TxHashsetValidation {
					kernels: ks,
					kernel_total: kt,
					rproofs: rps,
					rproof_total: rpt,
				};
			}
			_ => {
				*status = SyncStatus::TxHashsetValidation {
					kernels: 0,
					kernel_total: 0,
					rproofs: 0,
					rproof_total: 0,
				}
			}
		}
	}

	fn on_save(&self) {
		self.update(SyncStatus::TxHashsetSave);
	}

	fn on_done(&self) {
		self.update(SyncStatus::TxHashsetDone);
	}
}

/// A helper to hold the roots of the txhashset in order to keep them
/// readable.
#[derive(Debug, PartialEq)]
pub struct TxHashSetRoots {
	/// Output root
	pub output_root: Hash,
	/// Range Proof root
	pub rproof_root: Hash,
	/// Kernel root
	pub kernel_root: Hash,
}

/// A helper to hold the output pmmr position of the txhashset in order to keep them
/// readable.
#[derive(Debug)]
pub struct OutputMMRPosition {
	/// The hash at the output position in the MMR.
	pub output_mmr_hash: Hash,
	/// MMR position
	pub position: u64,
	/// Block height
	pub height: u64,
}

/// The tip of a fork. A handle to the fork ancestry from its leaf in the
/// blockchain tree. References the max height and the latest and previous
/// blocks
/// for convenience and the total difficulty.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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
			total_difficulty: Difficulty::min(),
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
	fn read(reader: &mut dyn ser::Reader) -> Result<Tip, ser::Error> {
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
	fn on_setup(&self);
	/// Starting validation
	fn on_validation(&self, kernels: u64, kernel_total: u64, rproofs: u64, rproof_total: u64);
	/// Starting to save the txhashset and related data
	fn on_save(&self);
	/// Done writing a new txhashset
	fn on_done(&self);
}

/// Do-nothing implementation of TxHashsetWriteStatus
pub struct NoStatus;

impl TxHashsetWriteStatus for NoStatus {
	fn on_setup(&self) {}
	fn on_validation(&self, _ks: u64, _kts: u64, _rs: u64, _rt: u64) {}
	fn on_save(&self) {}
	fn on_done(&self) {}
}

/// Dummy adapter used as a placeholder for real implementations
pub struct NoopAdapter {}

impl ChainAdapter for NoopAdapter {
	fn block_accepted(&self, _b: &Block, _status: BlockStatus, _opts: Options) {}
}

/// Status of an accepted block.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockStatus {
	/// Block is the "next" block, updating the chain head.
	Next,
	/// Block does not update the chain head and is a fork.
	Fork,
	/// Block updates the chain head via a (potentially disruptive) "reorg".
	/// Previous block was not our previous chain head.
	Reorg(u64),
}
