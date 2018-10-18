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

use consensus::HeaderInfo;
use consensus::{
	graph_weight, BASE_EDGE_BITS, BLOCK_TIME_SEC, COINBASE_MATURITY, CUT_THROUGH_HORIZON,
	DAY_HEIGHT, DIFFICULTY_ADJUST_WINDOW, INITIAL_DIFFICULTY, PROOFSIZE, SECOND_POW_EDGE_BITS,
	UNIT_DIFFICULTY
};
use pow::{self, CuckatooContext, EdgeType, PoWContext};
/// An enum collecting sets of parameters used throughout the
/// code wherever mining is needed. This should allow for
/// different sets of parameters for different purposes,
/// e.g. CI, User testing, production values
use std::sync::RwLock;

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
pub const TESTING_CUT_THROUGH_HORIZON: u32 = 20;

/// Testing initial graph weight
pub const TESTING_INITIAL_GRAPH_WEIGHT: u32 = 1;

/// Testing initial block difficulty
pub const TESTING_INITIAL_DIFFICULTY: u64 = 1;

/// Testnet 2 initial block difficulty, high to see how it goes
pub const TESTNET2_INITIAL_DIFFICULTY: u64 = 1000;

/// Testnet 3 initial block difficulty, moderately high, taking into account
/// a 30x Cuckoo adjustment factor
pub const TESTNET3_INITIAL_DIFFICULTY: u64 = 30000;

/// If a peer's last updated difficulty is 2 hours ago and its difficulty's lower than ours,
/// we're sure this peer is a stuck node, and we will kick out such kind of stuck peers.
pub const STUCK_PEER_KICK_TIME: i64 = 2 * 3600 * 1000;

/// Testnet 4 initial block difficulty
/// 1_000 times natural scale factor for cuckatoo29
pub const TESTNET4_INITIAL_DIFFICULTY: u64 = 1_000 * UNIT_DIFFICULTY;

/// Trigger compaction check on average every day for FAST_SYNC_NODE,
/// roll the dice on every block to decide,
/// all blocks lower than (BodyHead.height - CUT_THROUGH_HORIZON) will be removed.
pub const COMPACTION_CHECK: u64 = DAY_HEIGHT;

/// Types of chain a server can run with, dictates the genesis block and
/// and mining parameters used.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChainTypes {
	/// For CI testing
	AutomatedTesting,
	/// For User testing
	UserTesting,
	/// First test network
	Testnet1,
	/// Second test network
	Testnet2,
	/// Third test network
	Testnet3,
	/// Fourth test network
	Testnet4,
	/// Main production network
	Mainnet,
}

impl Default for ChainTypes {
	fn default() -> ChainTypes {
		ChainTypes::Testnet4
	}
}

/// PoW test mining and verifier context
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PoWContextTypes {
	/// Classic Cuckoo
	Cuckoo,
	/// Bleeding edge Cuckatoo
	Cuckatoo,
}

lazy_static!{
	/// The mining parameter mode
	pub static ref CHAIN_TYPE: RwLock<ChainTypes> =
			RwLock::new(ChainTypes::Mainnet);

	/// PoW context type to instantiate
	pub static ref POW_CONTEXT_TYPE: RwLock<PoWContextTypes> =
			RwLock::new(PoWContextTypes::Cuckoo);
}

/// Set the mining mode
pub fn set_mining_mode(mode: ChainTypes) {
	let mut param_ref = CHAIN_TYPE.write().unwrap();
	*param_ref = mode;
}

/// Return either a cuckoo context or a cuckatoo context
/// Single change point
pub fn create_pow_context<T>(
	edge_bits: u8,
	proof_size: usize,
	max_sols: u32,
) -> Result<Box<impl PoWContext<T>>, pow::Error>
where
	T: EdgeType,
{
	CuckatooContext::<T>::new(edge_bits, proof_size, max_sols)
}

/// Return the type of the pos
pub fn pow_type() -> PoWContextTypes {
	PoWContextTypes::Cuckatoo
}

/// The minimum acceptable edge_bits
pub fn min_edge_bits() -> u8 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_MIN_EDGE_BITS,
		ChainTypes::UserTesting => USER_TESTING_MIN_EDGE_BITS,
		ChainTypes::Testnet1 => USER_TESTING_MIN_EDGE_BITS,
		_ => SECOND_POW_EDGE_BITS,
	}
}

/// Reference edge_bits used to compute factor on higher Cuck(at)oo graph sizes,
/// while the min_edge_bits can be changed on a soft fork, changing
/// base_edge_bits is a hard fork.
pub fn base_edge_bits() -> u8 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_MIN_EDGE_BITS,
		ChainTypes::UserTesting => USER_TESTING_MIN_EDGE_BITS,
		ChainTypes::Testnet1 => USER_TESTING_MIN_EDGE_BITS,
		_ => BASE_EDGE_BITS,
	}
}

/// The proofsize
pub fn proofsize() -> usize {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_PROOF_SIZE,
		ChainTypes::UserTesting => USER_TESTING_PROOF_SIZE,
		_ => PROOFSIZE,
	}
}

/// Coinbase maturity for coinbases to be spent
pub fn coinbase_maturity() -> u64 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_COINBASE_MATURITY,
		ChainTypes::UserTesting => USER_TESTING_COINBASE_MATURITY,
		_ => COINBASE_MATURITY,
	}
}

/// Initial mining difficulty
pub fn initial_block_difficulty() -> u64 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => TESTING_INITIAL_DIFFICULTY,
		ChainTypes::UserTesting => TESTING_INITIAL_DIFFICULTY,
		ChainTypes::Testnet1 => TESTING_INITIAL_DIFFICULTY,
		ChainTypes::Testnet2 => TESTNET2_INITIAL_DIFFICULTY,
		ChainTypes::Testnet3 => TESTNET3_INITIAL_DIFFICULTY,
		ChainTypes::Testnet4 => TESTNET4_INITIAL_DIFFICULTY,
		ChainTypes::Mainnet => INITIAL_DIFFICULTY,
	}
}
/// Initial mining secondary scale
pub fn initial_graph_weight() -> u32 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => TESTING_INITIAL_GRAPH_WEIGHT,
		ChainTypes::UserTesting => TESTING_INITIAL_GRAPH_WEIGHT,
		ChainTypes::Testnet1 => TESTING_INITIAL_GRAPH_WEIGHT,
		ChainTypes::Testnet2 => TESTING_INITIAL_GRAPH_WEIGHT,
		ChainTypes::Testnet3 => TESTING_INITIAL_GRAPH_WEIGHT,
		ChainTypes::Testnet4 => graph_weight(SECOND_POW_EDGE_BITS) as u32,
		ChainTypes::Mainnet => graph_weight(SECOND_POW_EDGE_BITS) as u32,
	}
}

/// Horizon at which we can cut-through and do full local pruning
pub fn cut_through_horizon() -> u32 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => TESTING_CUT_THROUGH_HORIZON,
		ChainTypes::UserTesting => TESTING_CUT_THROUGH_HORIZON,
		_ => CUT_THROUGH_HORIZON,
	}
}

/// Are we in automated testing mode?
pub fn is_automated_testing_mode() -> bool {
	let param_ref = CHAIN_TYPE.read().unwrap();
	ChainTypes::AutomatedTesting == *param_ref
}

/// Are we in user testing mode?
pub fn is_user_testing_mode() -> bool {
	let param_ref = CHAIN_TYPE.read().unwrap();
	ChainTypes::UserTesting == *param_ref
}

/// Are we in production mode (a live public network)?
pub fn is_production_mode() -> bool {
	let param_ref = CHAIN_TYPE.read().unwrap();
	ChainTypes::Testnet1 == *param_ref
		|| ChainTypes::Testnet2 == *param_ref
		|| ChainTypes::Testnet3 == *param_ref
		|| ChainTypes::Testnet4 == *param_ref
		|| ChainTypes::Mainnet == *param_ref
}

/// Helper function to get a nonce known to create a valid POW on
/// the genesis block, to prevent it taking ages. Should be fine for now
/// as the genesis block POW solution turns out to be the same for every new
/// block chain at the moment
pub fn get_genesis_nonce() -> u64 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		// won't make a difference
		ChainTypes::AutomatedTesting => 0,
		// Magic nonce for current genesis block at cuckatoo15
		ChainTypes::UserTesting => 27944,
		// Magic nonce for genesis block for testnet2 (cuckatoo29)
		_ => panic!("Pre-set"),
	}
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

	// Sort blocks from earliest to latest (to keep conceptually easier)
	last_n.reverse();
	// Only needed just after blockchain launch... basically ensures there's
	// always enough data by simulating perfectly timed pre-genesis
	// blocks at the genesis difficulty as needed.
	let block_count_difference = needed_block_count - last_n.len();
	if block_count_difference > 0 {
		// Collect any real data we have
		let mut live_intervals: Vec<HeaderInfo> = last_n
			.iter()
			.map(|b| HeaderInfo::from_ts_diff(b.timestamp, b.difficulty))
			.collect();
		for i in (1..live_intervals.len()).rev() {
			// prevents issues with very fast automated test chains
			if live_intervals[i - 1].timestamp > live_intervals[i].timestamp {
				live_intervals[i].timestamp = 0;
			} else {
				live_intervals[i].timestamp =
					live_intervals[i].timestamp - live_intervals[i - 1].timestamp;
			}
		}
		// Remove genesis "interval"
		if live_intervals.len() > 1 {
			live_intervals.remove(0);
		} else {
			//if it's just genesis, adjust the interval
			live_intervals[0].timestamp = BLOCK_TIME_SEC;
		}
		let mut interval_index = live_intervals.len() - 1;
		let mut last_ts = last_n.first().unwrap().timestamp;
		let last_diff = live_intervals[live_intervals.len() - 1].difficulty;
		// fill in simulated blocks with values from the previous real block

		for _ in 0..block_count_difference {
			last_ts = last_ts.saturating_sub(live_intervals[live_intervals.len() - 1].timestamp);
			last_n.insert(0, HeaderInfo::from_ts_diff(last_ts, last_diff.clone()));
			interval_index = match interval_index {
				0 => live_intervals.len() - 1,
				_ => interval_index - 1,
			};
		}
	}
	last_n
}
