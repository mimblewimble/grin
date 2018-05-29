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

//! Server types

use std::convert::From;

use api;
use chain;
use core::core;
use core::global::ChainTypes;
use core::pow;
use p2p;
use pool;
use store;
use wallet;

/// Dandelion relay timer
const DANDELION_RELAY_SECS: u64 = 600;

/// Dandelion emabargo timer
const DANDELION_EMBARGO_SECS: u64 = 180;

/// Dandelion patience timer
const DANDELION_PATIENCE_SECS: u64 = 10;

/// Dandelion stem probability (stem 90% of the time, fluff 10%).
const DANDELION_STEM_PROBABILITY: usize = 90;

/// Error type wrapping underlying module errors.
#[derive(Debug)]
pub enum Error {
	/// Error originating from the core implementation.
	Core(core::block::Error),
	/// Error originating from the db storage.
	Store(store::Error),
	/// Error originating from the blockchain implementation.
	Chain(chain::Error),
	/// Error originating from the peer-to-peer network.
	P2P(p2p::Error),
	/// Error originating from HTTP API calls.
	API(api::Error),
	/// Error originating from wallet API.
	Wallet(wallet::Error),
	/// Error originating from the cuckoo miner
	Cuckoo(pow::cuckoo::Error),
}

impl From<core::block::Error> for Error {
	fn from(e: core::block::Error) -> Error {
		Error::Core(e)
	}
}
impl From<chain::Error> for Error {
	fn from(e: chain::Error) -> Error {
		Error::Chain(e)
	}
}

impl From<p2p::Error> for Error {
	fn from(e: p2p::Error) -> Error {
		Error::P2P(e)
	}
}

impl From<pow::cuckoo::Error> for Error {
	fn from(e: pow::cuckoo::Error) -> Error {
		Error::Cuckoo(e)
	}
}

impl From<store::Error> for Error {
	fn from(e: store::Error) -> Error {
		Error::Store(e)
	}
}

impl From<api::Error> for Error {
	fn from(e: api::Error) -> Error {
		Error::API(e)
	}
}

impl From<wallet::Error> for Error {
	fn from(e: wallet::Error) -> Error {
		Error::Wallet(e)
	}
}

/// Type of seeding the server will use to find other peers on the network.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChainValidationMode {
	/// Run full chain validation after processing every block.
	EveryBlock,
	/// Do not automatically run chain validation during normal block
	/// processing.
	Disabled,
}

impl Default for ChainValidationMode {
	fn default() -> ChainValidationMode {
		ChainValidationMode::Disabled
	}
}

/// Type of seeding the server will use to find other peers on the network.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Seeding {
	/// No seeding, mostly for tests that programmatically connect
	None,
	/// A list of seed addresses provided to the server
	List,
	/// Automatically download a text file with a list of server addresses
	WebStatic,
	/// Automatically get a list of seeds from mutliple DNS
	DNSSeed,
	/// Mostly for tests, where connections are initiated programmatically
	Programmatic,
}

impl Default for Seeding {
	fn default() -> Seeding {
		Seeding::DNSSeed
	}
}

fn default_dandelion_stem_probability() -> usize {
	DANDELION_STEM_PROBABILITY
}

fn default_dandelion_relay_secs() -> u64 {
	DANDELION_RELAY_SECS
}

fn default_dandelion_embargo_secs() -> u64 {
	DANDELION_EMBARGO_SECS
}

fn default_dandelion_patience_secs() -> u64 {
	DANDELION_PATIENCE_SECS
}

/// Dandelion config.
/// Note: Used by both p2p and pool components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DandelionConfig {
	/// Choose new Dandelion relay peer every n secs.
	#[serde = "default_dandelion_relay_secs"]
	pub relay_secs: u64,
	/// Dandelion embargo, fluff and broadcast tx if not seen on network before
	/// embargo expires.
	#[serde = "default_dandelion_embargo_secs"]
	pub embargo_secs: u64,
	/// Dandelion patience timer, fluff/stem processing runs every n secs.
	/// Tx aggregation happens on stem txs received within this window.
	#[serde = "default_dandelion_patience_secs"]
	pub patience_secs: u64,
	/// Dandelion stem probability.
	#[serde = "default_dandelion_stem_probability"]
	pub stem_probability: usize,
}

/// Default address for peer-to-peer connections.
impl Default for DandelionConfig {
	fn default() -> DandelionConfig {
		DandelionConfig {
			relay_secs: default_dandelion_relay_secs(),
			embargo_secs: default_dandelion_embargo_secs(),
			patience_secs: default_dandelion_patience_secs(),
			stem_probability: default_dandelion_stem_probability(),
		}
	}
}

/// Full server configuration, aggregating configurations required for the
/// different components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
	/// Directory under which the rocksdb stores will be created
	pub db_root: String,

	/// Network address for the Rest API HTTP server.
	pub api_http_addr: String,

	/// Setup the server for tests, testnet or mainnet
	#[serde(default)]
	pub chain_type: ChainTypes,

	/// Whether this node is a full archival node or a fast-sync, pruned node
	pub archive_mode: Option<bool>,

	/// Automatically run full chain validation during normal block processing?
	#[serde(default)]
	pub chain_validation_mode: ChainValidationMode,

	/// Method used to get the list of seed nodes for initial bootstrap.
	#[serde(default)]
	pub seeding_type: Seeding,

	/// TODO - move this into p2p_config?
	/// The list of seed nodes, if using Seeding as a seed type
	pub seeds: Option<Vec<String>>,

	/// TODO - move this into p2p_config?
	/// Capabilities expose by this node, also conditions which other peers this
	/// node will have an affinity toward when connection.
	pub capabilities: p2p::Capabilities,

	/// Configuration for the peer-to-peer server
	pub p2p_config: p2p::P2PConfig,

	/// Configuration for the mining daemon
	pub stratum_mining_config: Option<StratumServerConfig>,

	/// Transaction pool configuration
	#[serde(default)]
	pub pool_config: pool::PoolConfig,

	/// Dandelion configuration
	#[serde(default)]
	pub dandelion_config: DandelionConfig,

	/// Whether to skip the sync timeout on startup
	/// (To assist testing on solo chains)
	pub skip_sync_wait: Option<bool>,

	/// Whether to run the TUI
	/// if enabled, this will disable logging to stdout
	pub run_tui: Option<bool>,

	/// Whether to run the wallet listener with the server by default
	pub run_wallet_listener: Option<bool>,

	/// Whether to run the test miner (internal, cuckoo 16)
	pub run_test_miner: Option<bool>,

	/// Test miner wallet URL
	pub test_miner_wallet_url: Option<String>,
}

impl ServerConfig {
	/// Adapter for configuring Dandelion on the pool component.
	pub fn pool_dandelion_config(&self) -> pool::DandelionConfig {
		pool::DandelionConfig {
			relay_secs: self.dandelion_config.relay_secs,
			embargo_secs: self.dandelion_config.embargo_secs,
			patience_secs: self.dandelion_config.patience_secs,
			stem_probability: self.dandelion_config.stem_probability,
		}
	}

	/// Adapter for configuring Dandelion on the p2p component.
	pub fn p2p_dandelion_config(&self) -> p2p::DandelionConfig {
		p2p::DandelionConfig {
			relay_secs: self.dandelion_config.relay_secs,
			embargo_secs: self.dandelion_config.embargo_secs,
			patience_secs: self.dandelion_config.patience_secs,
			stem_probability: self.dandelion_config.stem_probability,
		}
	}
}

impl Default for ServerConfig {
	fn default() -> ServerConfig {
		ServerConfig {
			db_root: ".grin".to_string(),
			api_http_addr: "0.0.0.0:13413".to_string(),
			capabilities: p2p::Capabilities::FULL_NODE,
			seeding_type: Seeding::default(),
			seeds: None,
			p2p_config: p2p::P2PConfig::default(),
			dandelion_config: DandelionConfig::default(),
			stratum_mining_config: Some(StratumServerConfig::default()),
			chain_type: ChainTypes::default(),
			archive_mode: None,
			chain_validation_mode: ChainValidationMode::default(),
			pool_config: pool::PoolConfig::default(),
			skip_sync_wait: None,
			run_tui: None,
			run_wallet_listener: Some(false),
			run_test_miner: Some(false),
			test_miner_wallet_url: None,
		}
	}
}

/// Stratum (Mining server) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StratumServerConfig {
	/// Run a stratum mining server (the only way to communicate to mine this
	/// node via grin-miner
	pub enable_stratum_server: Option<bool>,

	/// If enabled, the address and port to listen on
	pub stratum_server_addr: Option<String>,

	/// How long to wait before stopping the miner, recollecting transactions
	/// and starting again
	pub attempt_time_per_block: u32,

	/// Base address to the HTTP wallet receiver
	pub wallet_listener_url: String,

	/// Attributes the reward to a random private key instead of contacting the
	/// wallet receiver. Mostly used for tests.
	pub burn_reward: bool,
}

impl Default for StratumServerConfig {
	fn default() -> StratumServerConfig {
		StratumServerConfig {
			wallet_listener_url: "http://localhost:13415".to_string(),
			burn_reward: false,
			attempt_time_per_block: 2,
			enable_stratum_server: None,
			stratum_server_addr: None,
		}
	}
}
