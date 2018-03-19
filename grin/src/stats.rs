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

use std::sync::{Arc, RwLock};
use std::sync::atomic::AtomicBool;

use chain;
use p2p;
use pow;

/// Server state info collection struct, to be passed around into internals
/// and populated when required
#[derive(Clone)]
pub struct ServerStateInfo {
	/// whether we're in a state of waiting for peers at startup
	pub awaiting_peers: Arc<AtomicBool>,
	/// Mining stats
	pub mining_stats: Arc<RwLock<MiningStats>>,
}

impl Default for ServerStateInfo {
	fn default() -> ServerStateInfo {
		ServerStateInfo {
			awaiting_peers: Arc::new(AtomicBool::new(false)),
			mining_stats: Arc::new(RwLock::new(MiningStats::default())),
		}
	}
}
/// Simpler thread-unware version of above to be populated and retured to
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
	pub is_syncing: bool,
	/// Whether we're awaiting peers
	pub awaiting_peers: bool,
	/// Handle to current mining stats
	pub mining_stats: MiningStats,
	/// Peer stats
	pub peer_stats: Vec<PeerStats>,
	/// Difficulty calculation statistics
	pub diff_stats: DiffStats,
}

/// Struct to return relevant information about the mining process
/// back to interested callers (such as the TUI)
#[derive(Clone)]
pub struct MiningStats {
	/// whether mining is enabled
	pub is_enabled: bool,
	/// whether we're currently mining
	pub is_mining: bool,
	/// combined graphs per second
	pub combined_gps: f64,
	/// what block height we're mining at
	pub block_height: u64,
	/// current network difficulty we're working on
	pub network_difficulty: u64,
	/// cuckoo size used for mining
	pub cuckoo_size: u16,
	/// Individual device status from Cuckoo-Miner
	pub device_stats: Option<Vec<Vec<pow::cuckoo_miner::CuckooMinerDeviceStats>>>,
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
	/// Ordinal index from current block
	pub block_index: i64,
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
	/// version running
	pub total_difficulty: u64,
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
			total_difficulty: peer.info.total_difficulty.into_num(),
			direction: direction.to_string(),
		}
	}
}

impl Default for MiningStats {
	fn default() -> MiningStats {
		MiningStats {
			is_enabled: false,
			is_mining: false,
			combined_gps: 0.0,
			block_height: 0,
			network_difficulty: 0,
			cuckoo_size: 0,
			device_stats: None,
		}
	}
}
