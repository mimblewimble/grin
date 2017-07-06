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
use store;

/// Error type wrapping underlying module errors.
#[derive(Debug)]
pub enum Error {
	/// Error originating from the db storage.
	Store(store::Error),
	/// Error originating from the blockchain implementation.
	Chain(chain::Error),
	/// Error originating from the peer-to-peer network.
	P2P(p2p::Error),
	/// Error originating from HTTP API calls
	API(api::Error),
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

/// Type of seeding the server will use to find other peers on the network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Seeding {
	/// No seeding, mostly for tests that programmatically connect
	None,
	/// A list of seed addresses provided to the server
	List(Vec<String>),
	/// Automatically download a text file with a list of server addresses
	WebStatic,
}

/// Full server configuration, aggregating configurations required for the
/// different components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
	/// Directory under which the rocksdb stores will be created
	pub db_root: String,

	/// Network address for the Rest API HTTP server.
	pub api_http_addr: String,

	/// Capabilities expose by this node, also conditions which other peers this
	/// node will have an affinity toward when connection.
	pub capabilities: p2p::Capabilities,

	/// Method used to get the list of seed nodes for initial bootstrap.
	pub seeding_type: Seeding,

	/// Configuration for the peer-to-peer server
	pub p2p_config: p2p::P2PConfig,

	/// Configuration for the mining daemon
	pub mining_config: MinerConfig,

	/// Setup the server for tests and testnet
	pub test_mode: bool,
}

/// Mining configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerConfig {
	/// Whether to start the miner with the server
	pub enable_mining: bool,

	/// Base address to the HTTP wallet receiver
	pub wallet_receiver_url: String,

	/// Attributes the reward to a random private key instead of contacting the
	/// wallet receiver. Mostly used for tests.
	pub burn_reward: bool,

	/// a testing attribute for the time being that artifically slows down the
	/// mining loop by adding a sleep to the thread
	pub slow_down_in_millis: u64,

	/// Size of Cuckoo Cycle to mine on
	pub cuckoo_size: u32,


}

impl Default for ServerConfig {
	fn default() -> ServerConfig {
		ServerConfig {
			db_root: ".grin".to_string(),
			api_http_addr: "127.0.0.1:13415".to_string(),
			capabilities: p2p::FULL_NODE,
			seeding_type: Seeding::None,
			p2p_config: p2p::P2PConfig::default(),
			mining_config: MinerConfig::default(),
			test_mode: true,
		}
	}
}

impl Default for MinerConfig {
	fn default() -> MinerConfig {
		MinerConfig {
			enable_mining: false,
			wallet_receiver_url: "http://localhost:13416".to_string(),
			burn_reward: false,
			slow_down_in_millis: 0,
			cuckoo_size: 0
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
	pub peer_count:u32,
	pub head: chain::Tip,
}
