// Copyright 2016 The Grin Developers
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

use std::convert::From;

use api;
use chain;
use p2p;
use pool;
use store;
use pow;
use wallet;
use core::global::MiningParameterMode;

/// Error type wrapping underlying module errors.
#[derive(Debug)]
pub enum Error {
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
pub enum Seeding {
	/// No seeding, mostly for tests that programmatically connect
	None,
	/// A list of seed addresses provided to the server
	List,
	/// Automatically download a text file with a list of server addresses
	WebStatic,
	/// Mostly for tests, where connections are initiated programmatically
	Programmatic,
}

/// Full server configuration, aggregating configurations required for the
/// different components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
	/// Directory under which the rocksdb stores will be created
	pub db_root: String,

	/// Network address for the Rest API HTTP server.
	pub api_http_addr: String,

	/// Setup the server for tests and testnet
	pub mining_parameter_mode: Option<MiningParameterMode>,

	/// Method used to get the list of seed nodes for initial bootstrap.
	pub seeding_type: Seeding,

	/// The list of seed nodes, if using Seeding as a seed type
	pub seeds: Option<Vec<String>>,

	/// Capabilities expose by this node, also conditions which other peers this
	/// node will have an affinity toward when connection.
	pub capabilities: p2p::Capabilities,

	/// Configuration for the peer-to-peer server
	pub p2p_config: Option<p2p::P2PConfig>,

	/// Configuration for the mining daemon
	pub mining_config: Option<pow::types::MinerConfig>,

	/// Transaction pool configuration
	#[serde(default)]
	pub pool_config: pool::PoolConfig,
}

impl Default for ServerConfig {
	fn default() -> ServerConfig {
		ServerConfig {
			db_root: ".grin".to_string(),
			api_http_addr: "0.0.0.0:13413".to_string(),
			capabilities: p2p::FULL_NODE,
			seeding_type: Seeding::None,
			seeds: None,
			p2p_config: Some(p2p::P2PConfig::default()),
			mining_config: Some(pow::types::MinerConfig::default()),
			mining_parameter_mode: Some(MiningParameterMode::Production),
			pool_config: pool::PoolConfig::default(),
		}
	}
}

/// Thread-safe container to return all sever related stats that other
/// consumers might be interested in, such as test results
///
///
///
#[derive(Clone)]
pub struct ServerStats {
	/// Number of peers
	pub peer_count: u32,
	/// Chain head
	pub head: chain::Tip,
}
