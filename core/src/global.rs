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

//! Values that should be shared across all modules, without necessarily
//! having to pass them all over the place, but aren't consensus values.
//! should be used sparingly.

use crate::consensus::HeaderInfo;
use crate::consensus::{
	graph_weight, BASE_EDGE_BITS, BLOCK_TIME_SEC, COINBASE_MATURITY, CUT_THROUGH_HORIZON,
	DAY_HEIGHT, DEFAULT_MIN_EDGE_BITS, DIFFICULTY_ADJUST_WINDOW, INITIAL_DIFFICULTY, PROOFSIZE,
	SECOND_POW_EDGE_BITS, STATE_SYNC_THRESHOLD,
};
use crate::pow::{self, new_cuckaroo_ctx, new_cuckatoo_ctx, EdgeType, PoWContext};
/// An enum collecting sets of parameters used throughout the
/// code wherever mining is needed. This should allow for
/// different sets of parameters for different purposes,
/// e.g. CI, User testing, production values
use crate::util::RwLock;

/// Define these here, as they should be developer-set, not really tweakable
/// by users

/// Automated testing edge_bits
pub const AUTOMATED_TESTING_MIN_EDGE_BITS: u8 = 9;

/// Automated testing proof size
pub const AUTOMATED_TESTING_PROOF_SIZE: usize = 4;

/// User testing edge_bits
pub const USER_TESTING_MIN_EDGE_BITS: u8 = 15;

/// User testing proof size
pub const USER_TESTING_PROOF_SIZE: usize = 42;

/// Automated testing coinbase maturity
pub const AUTOMATED_TESTING_COINBASE_MATURITY: u64 = 3;

/// User testing coinbase maturity
pub const USER_TESTING_COINBASE_MATURITY: u64 = 3;

/// Testing cut through horizon in blocks
pub const TESTING_CUT_THROUGH_HORIZON: u32 = 70;

/// Testing state sync threshold in blocks
pub const TESTING_STATE_SYNC_THRESHOLD: u32 = 20;

/// Testing initial graph weight
pub const TESTING_INITIAL_GRAPH_WEIGHT: u32 = 1;

/// Testing initial block difficulty
pub const TESTING_INITIAL_DIFFICULTY: u64 = 1;

/// If a peer's last updated difficulty is 2 hours ago and its difficulty's lower than ours,
/// we're sure this peer is a stuck node, and we will kick out such kind of stuck peers.
pub const STUCK_PEER_KICK_TIME: i64 = 2 * 3600 * 1000;

/// If a peer's last seen time is 2 weeks ago we will forget such kind of defunct peers.
const PEER_EXPIRATION_DAYS: i64 = 7 * 2;

/// Constant that expresses defunct peer timeout in seconds to be used in checks.
pub const PEER_EXPIRATION_REMOVE_TIME: i64 = PEER_EXPIRATION_DAYS * 24 * 3600;

/// Trigger compaction check on average every day for all nodes.
/// Randomized per node - roll the dice on every block to decide.
/// Will compact the txhashset to remove pruned data.
/// Will also remove old blocks and associated data from the database.
/// For a node configured as "archival_mode = true" only the txhashset will be compacted.
pub const COMPACTION_CHECK: u64 = DAY_HEIGHT;

/// Types of chain a server can run with, dictates the genesis block and
/// and mining parameters used.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChainTypes {
	/// For CI testing
	AutomatedTesting,
	/// For User testing
	UserTesting,
	/// Protocol testing network
	Floonet,
	/// Main production network
	Mainnet,
}

impl ChainTypes {
	/// Short name representing the chain type ("floo", "main", etc.)
	pub fn shortname(&self) -> String {
		match *self {
			ChainTypes::AutomatedTesting => "auto".to_owned(),
			ChainTypes::UserTesting => "user".to_owned(),
			ChainTypes::Floonet => "floo".to_owned(),
			ChainTypes::Mainnet => "main".to_owned(),
		}
	}
}

impl Default for ChainTypes {
	fn default() -> ChainTypes {
		ChainTypes::Floonet
	}
}

/// PoW test mining and verifier context
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PoWContextTypes {
	/// Classic Cuckoo
	Cuckoo,
	/// ASIC-friendly Cuckatoo
	Cuckatoo,
	/// ASIC-resistant Cuckaroo
	Cuckaroo,
}

lazy_static! {
	/// The mining parameter mode
	pub static ref CHAIN_TYPE: RwLock<ChainTypes> =
			RwLock::new(ChainTypes::Mainnet);

	/// PoW context type to instantiate
	pub static ref POW_CONTEXT_TYPE: RwLock<PoWContextTypes> =
			RwLock::new(PoWContextTypes::Cuckoo);
}

/// Set the mining mode
pub fn set_mining_mode(mode: ChainTypes) {
	let mut param_ref = CHAIN_TYPE.write();
	*param_ref = mode;
}

/// Return either a cuckoo context or a cuckatoo context
/// Single change point
pub fn create_pow_context<T>(
	_height: u64,
	edge_bits: u8,
	proof_size: usize,
	max_sols: u32,
) -> Result<Box<dyn PoWContext<T>>, pow::Error>
where
	T: EdgeType + 'static,
{
	let chain_type = CHAIN_TYPE.read().clone();
	match chain_type {
		// Mainnet has Cuckaroo29 for AR and Cuckatoo30+ for AF
		ChainTypes::Mainnet if edge_bits == 29 => new_cuckaroo_ctx(edge_bits, proof_size),
		ChainTypes::Mainnet => new_cuckatoo_ctx(edge_bits, proof_size, max_sols),

		// Same for Floonet
		ChainTypes::Floonet if edge_bits == 29 => new_cuckaroo_ctx(edge_bits, proof_size),
		ChainTypes::Floonet => new_cuckatoo_ctx(edge_bits, proof_size, max_sols),

		// Everything else is Cuckatoo only
		_ => new_cuckatoo_ctx(edge_bits, proof_size, max_sols),
	}
}

/// The minimum acceptable edge_bits
pub fn min_edge_bits() -> u8 {
	let param_ref = CHAIN_TYPE.read();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_MIN_EDGE_BITS,
		ChainTypes::UserTesting => USER_TESTING_MIN_EDGE_BITS,
		_ => DEFAULT_MIN_EDGE_BITS,
	}
}

/// Reference edge_bits used to compute factor on higher Cuck(at)oo graph sizes,
/// while the min_edge_bits can be changed on a soft fork, changing
/// base_edge_bits is a hard fork.
pub fn base_edge_bits() -> u8 {
	let param_ref = CHAIN_TYPE.read();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_MIN_EDGE_BITS,
		ChainTypes::UserTesting => USER_TESTING_MIN_EDGE_BITS,
		_ => BASE_EDGE_BITS,
	}
}

/// The proofsize
pub fn proofsize() -> usize {
	let param_ref = CHAIN_TYPE.read();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_PROOF_SIZE,
		ChainTypes::UserTesting => USER_TESTING_PROOF_SIZE,
		_ => PROOFSIZE,
	}
}

/// Coinbase maturity for coinbases to be spent
pub fn coinbase_maturity() -> u64 {
	let param_ref = CHAIN_TYPE.read();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_COINBASE_MATURITY,
		ChainTypes::UserTesting => USER_TESTING_COINBASE_MATURITY,
		_ => COINBASE_MATURITY,
	}
}

/// Initial mining difficulty
pub fn initial_block_difficulty() -> u64 {
	let param_ref = CHAIN_TYPE.read();
	match *param_ref {
		ChainTypes::AutomatedTesting => TESTING_INITIAL_DIFFICULTY,
		ChainTypes::UserTesting => TESTING_INITIAL_DIFFICULTY,
		ChainTypes::Floonet => INITIAL_DIFFICULTY,
		ChainTypes::Mainnet => INITIAL_DIFFICULTY,
	}
}
/// Initial mining secondary scale
pub fn initial_graph_weight() -> u32 {
	let param_ref = CHAIN_TYPE.read();
	match *param_ref {
		ChainTypes::AutomatedTesting => TESTING_INITIAL_GRAPH_WEIGHT,
		ChainTypes::UserTesting => TESTING_INITIAL_GRAPH_WEIGHT,
		ChainTypes::Floonet => graph_weight(0, SECOND_POW_EDGE_BITS) as u32,
		ChainTypes::Mainnet => graph_weight(0, SECOND_POW_EDGE_BITS) as u32,
	}
}

/// Horizon at which we can cut-through and do full local pruning
pub fn cut_through_horizon() -> u32 {
	let param_ref = CHAIN_TYPE.read();
	match *param_ref {
		ChainTypes::AutomatedTesting => TESTING_CUT_THROUGH_HORIZON,
		ChainTypes::UserTesting => TESTING_CUT_THROUGH_HORIZON,
		_ => CUT_THROUGH_HORIZON,
	}
}

/// Threshold at which we can request a txhashset (and full blocks from)
pub fn state_sync_threshold() -> u32 {
	let param_ref = CHAIN_TYPE.read();
	match *param_ref {
		ChainTypes::AutomatedTesting => TESTING_STATE_SYNC_THRESHOLD,
		ChainTypes::UserTesting => TESTING_STATE_SYNC_THRESHOLD,
		_ => STATE_SYNC_THRESHOLD,
	}
}

/// Are we in automated testing mode?
pub fn is_automated_testing_mode() -> bool {
	let param_ref = CHAIN_TYPE.read();
	ChainTypes::AutomatedTesting == *param_ref
}

/// Are we in user testing mode?
pub fn is_user_testing_mode() -> bool {
	let param_ref = CHAIN_TYPE.read();
	ChainTypes::UserTesting == *param_ref
}

/// Are we in production mode?
/// Production defined as a live public network, testnet[n] or mainnet.
pub fn is_production_mode() -> bool {
	let param_ref = CHAIN_TYPE.read();
	ChainTypes::Floonet == *param_ref || ChainTypes::Mainnet == *param_ref
}

/// Are we in floonet?
pub fn is_floonet() -> bool {
	let param_ref = CHAIN_TYPE.read();
	ChainTypes::Floonet == *param_ref
}

/// Are we for real?
pub fn is_mainnet() -> bool {
	let param_ref = CHAIN_TYPE.read();
	ChainTypes::Mainnet == *param_ref
}

/// Helper function to get a nonce known to create a valid POW on
/// the genesis block, to prevent it taking ages. Should be fine for now
/// as the genesis block POW solution turns out to be the same for every new
/// block chain at the moment
pub fn get_genesis_nonce() -> u64 {
	let param_ref = CHAIN_TYPE.read();
	match *param_ref {
		// won't make a difference
		ChainTypes::AutomatedTesting => 0,
		// Magic nonce for current genesis block at cuckatoo15
		ChainTypes::UserTesting => 27944,
		// Placeholder, obviously not the right value
		ChainTypes::Floonet => 0,
		// Placeholder, obviously not the right value
		ChainTypes::Mainnet => 0,
	}
}

/// Short name representing the current chain type ("floo", "main", etc.)
pub fn chain_shortname() -> String {
	let param_ref = CHAIN_TYPE.read();
	param_ref.shortname()
}

/// Converts an iterator of block difficulty data to more a more manageable
/// vector and pads if needed (which will) only be needed for the first few
/// blocks after genesis

pub fn difficulty_data_to_vector<T>(cursor: T) -> Vec<HeaderInfo>
where
	T: IntoIterator<Item = HeaderInfo>,
{
	// Convert iterator to vector, so we can append to it if necessary
	let needed_block_count = DIFFICULTY_ADJUST_WINDOW as usize + 1;
	let mut last_n: Vec<HeaderInfo> = cursor.into_iter().take(needed_block_count).collect();

	// Only needed just after blockchain launch... basically ensures there's
	// always enough data by simulating perfectly timed pre-genesis
	// blocks at the genesis difficulty as needed.
	let n = last_n.len();
	if needed_block_count > n {
		let last_ts_delta = if n > 1 {
			last_n[0].timestamp - last_n[1].timestamp
		} else {
			BLOCK_TIME_SEC
		};
		let last_diff = last_n[0].difficulty;

		// fill in simulated blocks with values from the previous real block
		let mut last_ts = last_n.last().unwrap().timestamp;
		for _ in n..needed_block_count {
			last_ts = last_ts.saturating_sub(last_ts_delta);
			last_n.push(HeaderInfo::from_ts_diff(last_ts, last_diff.clone()));
		}
	}
	last_n.reverse();
	last_n
}
