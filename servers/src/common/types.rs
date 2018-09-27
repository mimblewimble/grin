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
use std::sync::{Arc, RwLock};

use api;
use chain;
use core::global::ChainTypes;
use core::{core, pow};
use p2p;
use pool;
use store;
use util::LOGGER;
use wallet;

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
	Cuckoo(pow::Error),
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

impl From<pow::Error> for Error {
	fn from(e: pow::Error) -> Error {
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

/// Full server configuration, aggregating configurations required for the
/// different components.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerConfig {
	/// Directory under which the rocksdb stores will be created
	pub db_root: String,

	/// Network address for the Rest API HTTP server.
	pub api_http_addr: String,

	/// Location of secret for basic auth on Rest API HTTP server.
	pub api_secret_path: Option<String>,

	/// Setup the server for tests, testnet or mainnet
	#[serde(default)]
	pub chain_type: ChainTypes,

	/// Automatically run full chain validation during normal block processing?
	#[serde(default)]
	pub chain_validation_mode: ChainValidationMode,

	/// Whether this node is a full archival node or a fast-sync, pruned node
	pub archive_mode: Option<bool>,

	/// Whether to skip the sync timeout on startup
	/// (To assist testing on solo chains)
	pub skip_sync_wait: Option<bool>,

	/// Whether to run the TUI
	/// if enabled, this will disable logging to stdout
	pub run_tui: Option<bool>,

	/// Whether to use the DB wallet backend implementation
	pub use_db_wallet: Option<bool>,

	/// Whether to run the test miner (internal, cuckoo 16)
	pub run_test_miner: Option<bool>,

	/// Test miner wallet URL
	pub test_miner_wallet_url: Option<String>,

	/// Configuration for the peer-to-peer server
	pub p2p_config: p2p::P2PConfig,

	/// Transaction pool configuration
	#[serde(default)]
	pub pool_config: pool::PoolConfig,

	/// Dandelion configuration
	#[serde(default)]
	pub dandelion_config: pool::DandelionConfig,

	/// Configuration for the mining daemon
	#[serde(default)]
	pub stratum_mining_config: Option<StratumServerConfig>,
}

impl ServerConfig {
	/// Configuration items validation check
	pub fn validation_check(&mut self) {
		// check [server.p2p_config.capabilities] with 'archive_mode' in [server]
		if let Some(archive) = self.archive_mode {
			// note: slog not available before config loaded, only print here.
			if archive != self
				.p2p_config
				.capabilities
				.contains(p2p::Capabilities::FULL_HIST)
			{
				// if conflict, 'archive_mode' win
				self.p2p_config
					.capabilities
					.toggle(p2p::Capabilities::FULL_HIST);
			}
		}

		// todo: other checks if needed
	}
}

impl Default for ServerConfig {
	fn default() -> ServerConfig {
		ServerConfig {
			db_root: "grin_chain".to_string(),
			api_http_addr: "127.0.0.1:13413".to_string(),
			api_secret_path: Some(".api_secret".to_string()),
			p2p_config: p2p::P2PConfig::default(),
			dandelion_config: pool::DandelionConfig::default(),
			stratum_mining_config: Some(StratumServerConfig::default()),
			chain_type: ChainTypes::default(),
			archive_mode: Some(false),
			chain_validation_mode: ChainValidationMode::default(),
			pool_config: pool::PoolConfig::default(),
			skip_sync_wait: Some(false),
			run_tui: Some(true),
			use_db_wallet: None,
			run_test_miner: Some(false),
			test_miner_wallet_url: None,
		}
	}
}

/// Stratum (Mining server) configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StratumServerConfig {
	/// Run a stratum mining server (the only way to communicate to mine this
	/// node via grin-miner
	pub enable_stratum_server: Option<bool>,

	/// If enabled, the address and port to listen on
	pub stratum_server_addr: Option<String>,

	/// How long to wait before stopping the miner, recollecting transactions
	/// and starting again
	pub attempt_time_per_block: u32,

	/// Minimum difficulty for worker shares
	pub minimum_share_difficulty: u64,

	/// Base address to the HTTP wallet receiver
	pub wallet_listener_url: String,

	/// Attributes the reward to a random private key instead of contacting the
	/// wallet receiver. Mostly used for tests.
	pub burn_reward: bool,
}

impl Default for StratumServerConfig {
	fn default() -> StratumServerConfig {
		StratumServerConfig {
			wallet_listener_url: "http://127.0.0.1:13415".to_string(),
			burn_reward: false,
			attempt_time_per_block: 15,
			minimum_share_difficulty: 1,
			enable_stratum_server: Some(false),
			stratum_server_addr: Some("127.0.0.1:13416".to_string()),
		}
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
	/// Downloading block headers
	HeaderSync {
		current_height: u64,
		highest_height: u64,
	},
	/// Downloading the various txhashsets
	TxHashsetDownload,
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
	/// Downloading blocks
	BodySync {
		current_height: u64,
		highest_height: u64,
	},
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
		*self.current.read().unwrap() != SyncStatus::NoSync
	}

	/// Current syncing status
	pub fn status(&self) -> SyncStatus {
		*self.current.read().unwrap()
	}

	/// Update the syncing status
	pub fn update(&self, new_status: SyncStatus) {
		if self.status() == new_status {
			return;
		}

		let mut status = self.current.write().unwrap();

		debug!(
			LOGGER,
			"sync_state: sync_status: {:?} -> {:?}", *status, new_status,
		);

		*status = new_status;
	}

	/// Communicate sync error
	pub fn set_sync_error(&self, error: Error) {
		*self.sync_error.write().unwrap() = Some(error);
	}

	/// Get sync error
	pub fn sync_error(&self) -> Arc<RwLock<Option<Error>>> {
		Arc::clone(&self.sync_error)
	}

	/// Clear sync error
	pub fn clear_sync_error(&self) {
		*self.sync_error.write().unwrap() = None;
	}
}

impl chain::TxHashsetWriteStatus for SyncState {
	fn on_setup(&self) {
		self.update(SyncStatus::TxHashsetSetup);
	}

	fn on_validation(&self, vkernels: u64, vkernel_total: u64, vrproofs: u64, vrproof_total: u64) {
		let mut status = self.current.write().unwrap();
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
		self.update(SyncStatus::BodySync {
			current_height: 0,
			highest_height: 0,
		});
	}
}
