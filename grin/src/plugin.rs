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

use core::pow::cuckoo;
use core::pow::cuckoo::Error;
use core::pow::MiningWorker;
use core::consensus::{TEST_SIZESHIFT, DEFAULT_SIZESHIFT};

use std::collections::HashMap;

use core::core::Proof;
use types::{MinerConfig, ServerConfig};

use cuckoo_miner::{
	CuckooMiner,
	CuckooPluginManager,
	CuckooMinerConfig,
	CuckooMinerError,
	CuckooMinerSolution,
	CuckooPluginCapabilities};

pub struct PluginMiner {
	miner:Option<CuckooMiner>,
	last_solution: CuckooMinerSolution,
}

impl Default for PluginMiner {
	fn default() -> PluginMiner {
		PluginMiner {
			miner: None,
			last_solution: CuckooMinerSolution::new(),
		}
	}
}

impl PluginMiner {
	pub fn init(&mut self, miner_config: MinerConfig, server_config: ServerConfig){
				//Get directory of executable
		let mut exe_path=env::current_exe().unwrap();
		exe_path.pop();
		let exe_path=exe_path.to_str().unwrap();

		let plugin_install_path = match miner_config.cuckoo_miner_plugin_dir {
			Some(s) => s,
			None => String::from(format!("{}/deps", exe_path))
		};

		let plugin_impl_filter = match miner_config.cuckoo_miner_plugin_type {
			Some(s) => s,
			None => String::from("simple")
		};

		//First, load and query the plugins in the given directory
		//These should all be stored in 'deps' at the moment relative
		//to the executable path, though they should appear somewhere else 
		//when packaging is more//thought out 

    	let mut plugin_manager = CuckooPluginManager::new().unwrap();
    	let result=plugin_manager.load_plugin_dir(plugin_install_path);

		if let Err(e) = result {
			error!("Unable to load cuckoo-miner plugin directory, either from configuration or [exe_path]/deps.");
			panic!("Unable to load plugin directory... Please check configuration values");
		}

    	//The miner implementation needs to match what's in the consensus sizeshift value
		//
		let sz = if server_config.test_mode {
			TEST_SIZESHIFT
		} else {
			DEFAULT_SIZESHIFT
		};

		//So this is built dynamically based on the plugin implementation
		//type and the consensus sizeshift
		let filter = format!("{}_{}", plugin_impl_filter, sz);

    	let caps = plugin_manager.get_available_plugins(&filter).unwrap();
		//insert it into the miner configuration being created below

    
    	let mut config = CuckooMinerConfig::new();

        info!("Mining using plugin: {}", caps[0].full_path.clone());
    	config.plugin_full_path = caps[0].full_path.clone();
		if let Some(l) = miner_config.cuckoo_miner_parameter_list {
			config.parameter_list = l.clone();
		}

		//this will load the associated plugin
		let result=CuckooMiner::new(config);
		if let Err(e) = result {
			error!("Error initializing mining plugin: {:?}", e);
			error!("Accepted values are: {:?}", caps[0].parameters);
			panic!("Unable to init mining plugin.");
		}
		self.miner=Some(result.unwrap());
	} 
}

impl MiningWorker for PluginMiner {

	/// This will initialise a plugin according to what's currently
	/// included in CONSENSUS::TEST_SIZESHIFT, just using the edgetrim
	/// version of the miner for now, though this should become
	/// configurable somehow

	fn new(ease: u32, 
		   sizeshift: u32) -> Self {
		PluginMiner::default()
	}

	/// And simply calls the mine function of the loaded plugin
	/// returning whether a solution was found and the solution itself

	fn mine(&mut self, header: &[u8]) -> Result<Proof, cuckoo::Error> {
        let result = self.miner.as_mut().unwrap().mine(&header, &mut self.last_solution).unwrap();
		if result == true {
            return Ok(Proof(self.last_solution.solution_nonces));
        }
        Err(Error::NoSolution)
	}
}

