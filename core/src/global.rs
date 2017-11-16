// Copyright 2017 The Grin Developers
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

/// An enum collecting sets of parameters used throughout the
/// code wherever mining is needed. This should allow for
/// different sets of parameters for different purposes,
/// e.g. CI, User testing, production values

use std::sync::RwLock;
use consensus::PROOFSIZE;
use consensus::DEFAULT_SIZESHIFT;
use consensus::COINBASE_MATURITY;

/// Define these here, as they should be developer-set, not really tweakable
/// by users

/// Automated testing sizeshift
pub const AUTOMATED_TESTING_SIZESHIFT: u8 = 10;

/// Automated testing proof size
pub const AUTOMATED_TESTING_PROOF_SIZE: usize = 4;

/// User testing sizeshift
pub const USER_TESTING_SIZESHIFT: u8 = 16;

/// User testing proof size
pub const USER_TESTING_PROOF_SIZE: usize = 42;

/// Automated testing coinbase maturity
pub const AUTOMATED_TESTING_COINBASE_MATURITY: u64 = 3;

/// User testing coinbase maturity
pub const USER_TESTING_COINBASE_MATURITY: u64 = 3;

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

  /// Main production network
	Mainnet,
}

impl Default for ChainTypes {
	fn default() -> ChainTypes {
		ChainTypes::UserTesting
	}
}

lazy_static!{
	/// The mining parameter mode
	pub static ref CHAIN_TYPE: RwLock<ChainTypes> =
			RwLock::new(ChainTypes::Mainnet);
}

/// Set the mining mode
pub fn set_mining_mode(mode: ChainTypes) {
	let mut param_ref = CHAIN_TYPE.write().unwrap();
	*param_ref = mode;
}

/// The sizeshift
pub fn sizeshift() -> u8 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_SIZESHIFT,
		ChainTypes::UserTesting => USER_TESTING_SIZESHIFT,
		ChainTypes::Testnet1 => USER_TESTING_SIZESHIFT,
		ChainTypes::Mainnet => DEFAULT_SIZESHIFT,
	}
}

/// The proofsize
pub fn proofsize() -> usize {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_PROOF_SIZE,
		ChainTypes::UserTesting => USER_TESTING_PROOF_SIZE,
		ChainTypes::Testnet1 => PROOFSIZE,
		ChainTypes::Mainnet => PROOFSIZE,
	}
}

/// Coinbase maturity
pub fn coinbase_maturity() -> u64 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		ChainTypes::AutomatedTesting => AUTOMATED_TESTING_COINBASE_MATURITY,
		ChainTypes::UserTesting => USER_TESTING_COINBASE_MATURITY,
		ChainTypes::Testnet1 => COINBASE_MATURITY,
		ChainTypes::Mainnet => COINBASE_MATURITY,
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
	ChainTypes::Testnet1 == *param_ref ||
    ChainTypes::Mainnet == *param_ref
}

/// Helper function to get a nonce known to create a valid POW on
/// the genesis block, to prevent it taking ages. Should be fine for now
/// as the genesis block POW solution turns out to be the same for every new
/// block chain
/// at the moment
pub fn get_genesis_nonce() -> u64 {
	let param_ref = CHAIN_TYPE.read().unwrap();
	match *param_ref {
		// won't make a difference
		ChainTypes::AutomatedTesting => 0,
		// Magic nonce for current genesis block at cuckoo16
		ChainTypes::UserTesting => 27944,

		_ => panic!("Pre-set"),
	}
}
