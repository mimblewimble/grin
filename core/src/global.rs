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

use consensus::TargetError;
use consensus::{
	BLOCK_TIME_SEC, COINBASE_MATURITY, CUT_THROUGH_HORIZON, DEFAULT_MIN_SIZESHIFT,
	DIFFICULTY_ADJUST_WINDOW, EASINESS, INITIAL_DIFFICULTY, MEDIAN_TIME_WINDOW, PROOFSIZE,
	REFERENCE_SIZESHIFT,
};
use pow::{self, CuckooContext, Difficulty, EdgeType, PoWContext};
/// An enum collecting sets of parameters used throughout the
/// code wherever mining is needed. This should allow for
/// different sets of parameters for different purposes,
/// e.g. CI, User testing, production values
use std::sync::RwLock;

/// Define these here, as they should be developer-set, not really tweakable
/// by users

/// Automated testing sizeshift
pub const AUTOMATED_TESTING_MIN_SIZESHIFT: u8 = 10;

/// Automated testing proof size
pub const AUTOMATED_TESTING_PROOF_SIZE: usize = 4;

/// User testing sizeshift
pub const USER_TESTING_MIN_SIZESHIFT: u8 = 16;

/// User testing proof size
pub const USER_TESTING_PROOF_SIZE: usize = 42;

/// Automated testing coinbase maturity
pub const AUTOMATED_TESTING_COINBASE_MATURITY: u64 = 3;

/// User testing coinbase maturity
pub const USER_TESTING_COINBASE_MATURITY: u64 = 3;

/// Old coinbase maturity
/// TODO: obsolete for mainnet together with maturity code below
pub const OLD_COINBASE_MATURITY: u64 = 1_000;
/// soft-fork around Sep 17 2018 on testnet3
pub const COINBASE_MATURITY_FORK_HEIGHT: u64 = 100_000;

/// Testing cut through horizon in blocks
pub const TESTING_CUT_THROUGH_HORIZON: u32 = 20;

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
	/// Main production network
	Mainnet,
}

impl Default for ChainTypes {
	fn default() -> ChainTypes {
		ChainTypes::Testnet3
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
	// Perform whatever tests, configuration etc are needed to determine desired context + edge size
	// + params
	// Hardcode to regular cuckoo for now
	CuckooContext::<T>::new(edge_bits, proof_size, EASINESS, max_sols)
	// Or switch to cuckatoo as follows:
	// CuckatooContext::<T>::new(edge_bits, proof_size, easiness_pct, max_sols)
}

/// The minimum acceptable sizeshift
pub fn min_sizeshift() -> u8 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_MIN_SIZESHIFT,
		ChainTypes::UserTesting => USER_TESTING_MIN_SIZESHIFT,
		ChainTypes::Testnet1 => USER_TESTING_MIN_SIZESHIFT,
		_ => DEFAULT_MIN_SIZESHIFT,
	}
}

/// Reference sizeshift used to compute factor on higher Cuckoo graph sizes,
/// while the min_sizeshift can be changed on a soft fork, changing
/// ref_sizeshift is a hard fork.
pub fn ref_sizeshift() -> u8 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_MIN_SIZESHIFT,
		ChainTypes::UserTesting => USER_TESTING_MIN_SIZESHIFT,
		ChainTypes::Testnet1 => USER_TESTING_MIN_SIZESHIFT,
		_ => REFERENCE_SIZESHIFT,
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

/// Coinbase maturity for coinbases to be spent at given height
pub fn coinbase_maturity(height: u64) -> u64 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_COINBASE_MATURITY,
		ChainTypes::UserTesting => USER_TESTING_COINBASE_MATURITY,
		_ => if height < COINBASE_MATURITY_FORK_HEIGHT {
			OLD_COINBASE_MATURITY
		} else {
			COINBASE_MATURITY
		},
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
		ChainTypes::Mainnet => INITIAL_DIFFICULTY,
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
		// Magic nonce for current genesis block at cuckoo16
		ChainTypes::UserTesting => 27944,
		// Magic nonce for genesis block for testnet2 (cuckoo30)
		_ => panic!("Pre-set"),
	}
}

/// Converts an iterator of block difficulty data to more a more manageable
/// vector and pads if needed (which will) only be needed for the first few
/// blocks after genesis

pub fn difficulty_data_to_vector<T>(cursor: T) -> Vec<Result<(u64, Difficulty), TargetError>>
where
	T: IntoIterator<Item = Result<(u64, Difficulty), TargetError>>,
{
	// Convert iterator to vector, so we can append to it if necessary
	let needed_block_count = (MEDIAN_TIME_WINDOW + DIFFICULTY_ADJUST_WINDOW) as usize;
	let mut last_n: Vec<Result<(u64, Difficulty), TargetError>> =
		cursor.into_iter().take(needed_block_count).collect();

	// Sort blocks from earliest to latest (to keep conceptually easier)
	last_n.reverse();
	// Only needed just after blockchain launch... basically ensures there's
	// always enough data by simulating perfectly timed pre-genesis
	// blocks at the genesis difficulty as needed.
	let block_count_difference = needed_block_count - last_n.len();
	if block_count_difference > 0 {
		// Collect any real data we have
		let mut live_intervals: Vec<(u64, Difficulty)> = last_n
			.iter()
			.map(|b| (b.clone().unwrap().0, b.clone().unwrap().1))
			.collect();
		for i in (1..live_intervals.len()).rev() {
			// prevents issues with very fast automated test chains
			if live_intervals[i - 1].0 > live_intervals[i].0 {
				live_intervals[i].0 = 0;
			} else {
				live_intervals[i].0 = live_intervals[i].0 - live_intervals[i - 1].0;
			}
		}
		// Remove genesis "interval"
		if live_intervals.len() > 1 {
			live_intervals.remove(0);
		} else {
			//if it's just genesis, adjust the interval
			live_intervals[0].0 = BLOCK_TIME_SEC;
		}
		let mut interval_index = live_intervals.len() - 1;
		let mut last_ts = last_n.first().as_ref().unwrap().as_ref().unwrap().0;
		let last_diff = live_intervals[live_intervals.len() - 1].1;
		// fill in simulated blocks with values from the previous real block

		for _ in 0..block_count_difference {
			last_ts = last_ts.saturating_sub(live_intervals[live_intervals.len() - 1].0);
			last_n.insert(0, Ok((last_ts, last_diff.clone())));
			interval_index = match interval_index {
				0 => live_intervals.len() - 1,
				_ => interval_index - 1,
			};
		}
	}
	last_n
}
