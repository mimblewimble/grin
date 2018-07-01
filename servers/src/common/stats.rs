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

//! Server stat collection types, to be used by tests, logging or GUI/TUI
//! to collect information about server status

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use chain;
use common::types::SyncStatus;
use p2p;

/// Server state info collection struct, to be passed around into internals
/// and populated when required
#[derive(Clone)]
pub struct ServerStateInfo {
	/// whether we're in a state of waiting for peers at startup
	pub awaiting_peers: Arc<AtomicBool>,
	/// Stratum stats
	pub stratum_stats: Arc<RwLock<StratumStats>>,
}

impl Default for ServerStateInfo {
	fn default() -> ServerStateInfo {
		ServerStateInfo {
			awaiting_peers: Arc::new(AtomicBool::new(false)),
			stratum_stats: Arc::new(RwLock::new(StratumStats::default())),
		}
	}
}
/// Simpler thread-unaware version of above to be populated and returned to
/// consumers might be interested in, such as test results or UI
#[derive(Clone)]
pub struct ServerStats {
	/// Number of peers
	pub peer_count: u32,
	/// Chain head
	pub head: chain::Tip,
	/// sync header head
	pub header_head: chain::Tip,
	/// Whether we're currently syncing
	pub sync_status: SyncStatus,
	/// Whether we're awaiting peers
	pub awaiting_peers: bool,
	/// Handle to current stratum server stats
	pub stratum_stats: StratumStats,
	/// Peer stats
	pub peer_stats: Vec<PeerStats>,
	/// Difficulty calculation statistics
	pub diff_stats: DiffStats,
}

/// Struct to return relevant information about stratum workers
#[derive(Clone, Serialize, Debug)]
pub struct WorkerStats {
	/// Unique ID for this worker
	pub id: String,
	/// whether stratum worker is currently connected
	pub is_connected: bool,
	/// Timestamp of most recent communication with this worker
	pub last_seen: SystemTime,
	/// pow difficulty this worker is using
	pub pow_difficulty: u64,
	/// number of valid shares submitted
	pub num_accepted: u64,
	/// number of invalid shares submitted
	pub num_rejected: u64,
	/// number of shares submitted too late
	pub num_stale: u64,
}

/// Struct to return relevant information about the stratum server
#[derive(Clone, Serialize, Debug)]
pub struct StratumStats {
	/// whether stratum server is enabled
	pub is_enabled: bool,
	/// whether stratum server is running
	pub is_running: bool,
	/// Number of connected workers
	pub num_workers: usize,
	/// what block height we're mining at
	pub block_height: u64,
	/// current network difficulty we're working on
	pub network_difficulty: u64,
	/// cuckoo size used for mining
	pub cuckoo_size: u16,
	/// Individual worker status
	pub worker_stats: Vec<WorkerStats>,
}

/// Stats on the last WINDOW blocks and the difficulty calculation
#[derive(Clone)]
pub struct DiffStats {
	/// latest height
	pub height: u64,
	/// Last WINDOW block data
	pub last_blocks: Vec<DiffBlock>,
	/// Average block time for last WINDOW blocks
	pub average_block_time: u64,
	/// Average WINDOW difficulty
	pub average_difficulty: u64,
	/// WINDOW size
	pub window_size: u64,
}

/// Last n blocks for difficulty calculation purposes
#[derive(Clone, Debug)]
pub struct DiffBlock {
	/// Block number (can be negative for a new chain)
	pub block_number: i64,
	/// Block network difficulty
	pub difficulty: u64,
	/// Time block was found (epoch seconds)
	pub time: u64,
	/// Duration since previous block (epoch seconds)
	pub duration: u64,
}

/// Struct to return relevant information about peers
#[derive(Clone, Debug)]
pub struct PeerStats {
	/// Current state of peer
	pub state: String,
	/// Address
	pub addr: String,
	/// version running
	pub version: u32,
	/// difficulty repored by peer
	pub total_difficulty: u64,
	/// height reported by peer on ping
	pub height: u64,
	/// direction
	pub direction: String,
}

impl PeerStats {
	/// Convert from a peer directly
	pub fn from_peer(peer: &p2p::Peer) -> PeerStats {
		// State
		let mut state = "Disconnected";
		if peer.is_connected() {
			state = "Connected";
		}
		if peer.is_banned() {
			state = "Banned";
		}
		let addr = peer.info.addr.to_string();
		let direction = match peer.info.direction {
			p2p::types::Direction::Inbound => "Inbound",
			p2p::types::Direction::Outbound => "Outbound",
		};
		PeerStats {
			state: state.to_string(),
			addr: addr,
			version: peer.info.version,
			total_difficulty: peer.info.total_difficulty.to_num(),
			height: peer.info.height,
			direction: direction.to_string(),
		}
	}
}

impl Default for WorkerStats {
	fn default() -> WorkerStats {
		WorkerStats {
			id: String::from("unknown"),
			is_connected: false,
			last_seen: SystemTime::now(),
			pow_difficulty: 0,
			num_accepted: 0,
			num_rejected: 0,
			num_stale: 0,
		}
	}
}

impl Default for StratumStats {
	fn default() -> StratumStats {
		StratumStats {
			is_enabled: false,
			is_running: false,
			num_workers: 0,
			block_height: 0,
			network_difficulty: 0,
			cuckoo_size: 0,
			worker_stats: Vec::new(),
		}
	}
}
