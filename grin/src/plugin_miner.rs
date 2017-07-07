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

use core::core::Proof;

use cuckoo_miner::{
	CuckooMiner,
	CuckooPluginManager,
	CuckooMinerConfig,
	CuckooMinerError,
	CuckooMinerSolution,
	CuckooPluginCapabilities};

pub struct PluginMiner {
	miner:CuckooMiner,
	last_solution: CuckooMinerSolution,
}

impl MiningWorker for PluginMiner {

	// Initialise the plugin parameters for now
	fn new(ease: u32, sizeshift: u32) -> Self {

		//Get directory of executable
		let mut exe_path=env::current_exe().unwrap();
		exe_path.pop();
		let exe_path=exe_path.to_str().unwrap();

		//First, load and query the plugins in the given directory
    	let mut plugin_manager = CuckooPluginManager::new().unwrap();
    	let result=plugin_manager.load_plugin_dir(String::from(format!("{}/deps", exe_path))).expect("");
    	//Get a list of installed plugins and capabilities.. filtering for the one we want, if any
    	let caps = plugin_manager.get_available_plugins("simple_12").unwrap();
		//Select a plugin somehow, and insert it into the miner configuration
    	//being created below
    
    	let mut config = CuckooMinerConfig::new();
		//Just load the first one for now, should be 'cuckoo12'
        println!("Mining using plugin: {}", caps[0].full_path.clone());
    	config.plugin_full_path = caps[0].full_path.clone();
		config.num_threads=1;
		config.num_trims=0;

		//this will load the associated plugin
		let miner = CuckooMiner::new(config).expect("");

		PluginMiner {
			miner: miner,
			last_solution: CuckooMinerSolution::new(),
		}
	}
	
	fn mine(&mut self, header: &[u8]) -> Result<Proof, cuckoo::Error> {
        let result = self.miner.mine(&header, &mut self.last_solution).unwrap();
		if result == true {
            return Ok(Proof(self.last_solution.solution_nonces));
        }
        Err(Error::NoSolution)
	}
}

