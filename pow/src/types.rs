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
//! Mining configuration type

use std::collections::HashMap;

/// CuckooMinerPlugin configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuckooMinerPluginConfig {
	/// The type of plugin to load (i.e. filters on filename)
	pub type_filter: String,

	/// device params
	pub device_parameters: Option<HashMap<String, HashMap<String, u32>>>,
}

impl Default for CuckooMinerPluginConfig {
	fn default() -> CuckooMinerPluginConfig {
		CuckooMinerPluginConfig {
			type_filter: String::new(),
			device_parameters: None,
		}
	}
}

/// Mining configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerConfig {
	/// Whether to start the miner with the server (requires using cuckoo-miner)
	pub enable_mining: bool,

	/// Whether to use the async version of mining
	pub miner_async_mode: Option<bool>,

	/// plugin dir
	pub miner_plugin_dir: Option<String>,

	/// Cuckoo miner plugin configuration, one for each plugin
	pub miner_plugin_config: Option<Vec<CuckooMinerPluginConfig>>,

	/// How long to wait before stopping the miner, recollecting transactions
	/// and starting again
	pub attempt_time_per_block: u32,

	/// Base address to the HTTP wallet receiver
	pub wallet_listener_url: String,

	/// Attributes the reward to a random private key instead of contacting the
	/// wallet receiver. Mostly used for tests.
	pub burn_reward: bool,

	/// a testing attribute for the time being that artifically slows down the
	/// mining loop by adding a sleep to the thread
	pub slow_down_in_millis: Option<u64>,
}

impl Default for MinerConfig {
	fn default() -> MinerConfig {
		MinerConfig {
			enable_mining: false,
			miner_async_mode: None,
			miner_plugin_dir: None,
			miner_plugin_config: None,
			wallet_listener_url: "http://localhost:13415".to_string(),
			burn_reward: false,
			slow_down_in_millis: Some(0),
			attempt_time_per_block: 2,
		}
	}
}
