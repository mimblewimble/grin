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

//! Plugin wrapper for cuckoo miner, implementing common traits
//! with the existing embedded miner. This is all included conditionally
//! for compatibility reasons with those who aren't interested in playing
//! with cuckoo-miner at present

use std::env;

use cuckoo;
use cuckoo::Error;
use MiningWorker;
use core::global;

use core::core::Proof;
use types::MinerConfig;
use util::LOGGER;

use std::sync::Mutex;

use cuckoo_miner::{CuckooMiner, CuckooMinerConfig, CuckooMinerDeviceStats, CuckooMinerError,
                   CuckooMinerSolution, CuckooPluginManager};

// For now, we're just going to keep a static reference around to the loaded
// config
// And not allow querying the plugin directory twice once a plugin has been
// selected
// This is to keep compatibility with multi-threaded testing, so that spawned
// testing threads don't try to load/unload the library while another thread is
// using it.
lazy_static!{
	static ref LOADED_CONFIG: Mutex<Option<Vec<CuckooMinerConfig>>> = Mutex::new(None);
}

/// plugin miner
pub struct PluginMiner {
	/// the miner
	pub miner: Option<CuckooMiner>,
	last_solution: CuckooMinerSolution,
	config: Vec<CuckooMinerConfig>,
}

impl Default for PluginMiner {
	fn default() -> PluginMiner {
		PluginMiner {
			miner: None,
			config: Vec::new(),
			last_solution: CuckooMinerSolution::new(),
		}
	}
}

impl PluginMiner {
	/// Init the plugin miner
	pub fn init(&mut self, miner_config: MinerConfig) {
		// Get directory of executable
		let mut exe_path = env::current_exe().unwrap();
		exe_path.pop();
		let exe_path = exe_path.to_str().unwrap();
		let plugin_install_path = match miner_config.cuckoo_miner_plugin_dir.clone() {
			Some(s) => s,
			None => String::from(format!("{}/plugins", exe_path)),
		};

		let mut plugin_vec_filters = Vec::new();
		if let None = miner_config.cuckoo_miner_plugin_config {
			plugin_vec_filters.push(String::from("simple"));
		} else {
			for p in miner_config.clone().cuckoo_miner_plugin_config.unwrap() {
				plugin_vec_filters.push(p.type_filter);
			}
		}

		// First, load and query the plugins in the given directory
  // These should all be stored in 'plugins' at the moment relative
  // to the executable path, though they should appear somewhere else
  // when packaging is more//thought out

		let mut loaded_config_ref = LOADED_CONFIG.lock().unwrap();

		// Load from here instead
		if let Some(ref c) = *loaded_config_ref {
			debug!(LOGGER, "Not re-loading plugin or directory.");
			// this will load the associated plugin
			let result = CuckooMiner::new(c.clone());
			self.miner = Some(result.unwrap());
			self.config = c.clone();
			return;
		}

		let mut plugin_manager = CuckooPluginManager::new().unwrap();
		let result = plugin_manager.load_plugin_dir(plugin_install_path);

		if let Err(_) = result {
			error!(
				LOGGER,
				"Unable to load cuckoo-miner plugin directory, either from configuration or [exe_path]/plugins."
			);
			panic!("Unable to load plugin directory... Please check configuration values");
		}

		let sz = global::sizeshift();

		let mut cuckoo_configs = Vec::new();
		let mut index = 0;
		for f in plugin_vec_filters {
			// So this is built dynamically based on the plugin implementation
   // type and the consensus sizeshift
			let filter = format!("{}_{}", f, sz);

			let caps = plugin_manager.get_available_plugins(&filter).unwrap();
			// insert it into the miner configuration being created below

			let mut config = CuckooMinerConfig::new();

			info!(
				LOGGER,
				"Mining plugin {} - {}",
				index,
				caps[0].full_path.clone()
			);
			config.plugin_full_path = caps[0].full_path.clone();
			if let Some(l) = miner_config.clone().cuckoo_miner_plugin_config {
				if let Some(lp) = l[index].parameter_list.clone() {
					config.parameter_list = lp.clone();
				}
			}
			cuckoo_configs.push(config);
			index += 1;
		}
		// Store this config now, because we just want one instance
  // of the plugin lib per invocation now
		*loaded_config_ref = Some(cuckoo_configs.clone());

		// this will load the associated plugin
		let result = CuckooMiner::new(cuckoo_configs.clone());
		if let Err(e) = result {
			error!(LOGGER, "Error initializing mining plugin: {:?}", e);
			// error!(LOGGER, "Accepted values are: {:?}", caps[0].parameters);
			panic!("Unable to init mining plugin.");
		}

		self.config = cuckoo_configs.clone();
		self.miner = Some(result.unwrap());
	}

	/// Get the miner
	pub fn get_consumable(&mut self) -> CuckooMiner {
		// this will load the associated plugin
		let result = CuckooMiner::new(self.config.clone());
		if let Err(e) = result {
			error!(LOGGER, "Error initializing mining plugin: {:?}", e);
			panic!("Unable to init mining plugin.");
		}
		result.unwrap()
	}

	/// Returns the number of mining plugins that have been loaded
	pub fn loaded_plugin_count(&self) -> usize {
		self.config.len()
	}

	/// Get stats
	pub fn get_stats(&self, index: usize) -> Result<Vec<CuckooMinerDeviceStats>, CuckooMinerError> {
		self.miner.as_ref().unwrap().get_stats(index)
	}
}

impl MiningWorker for PluginMiner {
	/// This will initialise a plugin according to what's currently
	/// included in CONSENSUS::TEST_SIZESHIFT, just using the edgetrim
	/// version of the miner for now, though this should become
	/// configurable somehow

	fn new(_ease: u32, _sizeshift: u32, _proof_size: usize) -> Self {
		PluginMiner::default()
	}

	/// And simply calls the mine function of the loaded plugin
	/// returning whether a solution was found and the solution itself

	fn mine(&mut self, header: &[u8]) -> Result<Proof, cuckoo::Error> {
		let result = self.miner
			.as_mut()
			.unwrap()
			.mine(&header, &mut self.last_solution, 0)
			.unwrap();
		if result == true {
			return Ok(Proof::new(self.last_solution.solution_nonces.to_vec()));
		}
		Err(Error::NoSolution)
	}
}
