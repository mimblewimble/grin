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

//! Main for building the genesis generation utility.

use std::fs;
use std::sync::Arc;

use chrono::prelude::Utc;
use chrono::Duration;
use curl;
use serde_json;

use cuckoo_miner as cuckoo;
use grin_chain as chain;
use grin_core as core;
use grin_miner_plugin as plugin;
use grin_store as store;
use grin_util as util;

use grin_core::core::verifier_cache::LruVerifierCache;
use grin_keychain::{ExtKeychain, Keychain, BlindingFactor};

static BCHAIN_INFO_URL: &str = "https://blockchain.info/latestblock";
static BCYPHER_URL: &str = "https://api.blockcypher.com/v1/btc/main";
static BCHAIR_URL: &str = "https://api.blockchair.com/bitcoin/blocks?limit=2";

fn main() {
	core::global::set_mining_mode(core::global::ChainTypes::Mainnet);

	// get the latest bitcoin hash
	let h1 = get_bchain_head();
	let h2 = get_bcypher_head();
	let h3 = get_bchair_head();
	if h1 != h2 || h1 != h3 {
		panic!(
			"Bitcoin chain head is inconsistent, please retry ({}, {}, {}).",
			h1, h2, h3
		);
	}
	println!("Using bitcoin block hash {}", h1);

	// build the basic parts of the genesis block header, perhaps some of this
	// can be moved to core
	let mut gen = core::genesis::genesis_main();
	gen.header.timestamp = Utc::now() + Duration::minutes(30);
	gen.header.prev_root = core::core::hash::Hash::from_hex(&h1).unwrap();
	println!("Built genesis:\n{:?}", gen);

	// TODO get the proper keychain and/or raw coinbase
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id = ExtKeychain::derive_key_id(0, 1, 0, 0, 0);
	let reward = core::libtx::reward::output(&keychain, &key_id, 0, 0).unwrap();
	gen = gen.with_reward(reward.0, reward.1);

	{
		// setup a tmp chain to set block header roots
		let tmp_chain = setup_chain(".grin.tmp", core::pow::mine_genesis_block().unwrap());
		tmp_chain.set_txhashset_roots(&mut gen).unwrap();
	}

	// mine a Cuckaroo29 block
	let plugin_path = "cuckaroo_mean_cuda_29.cuckooplugin";
	let plugin_lib = cuckoo::PluginLibrary::new(plugin_path).unwrap();
	let solver_ctx = plugin_lib.create_solver_ctx(&mut plugin_lib.get_default_params());

	let mut solver_sols = plugin::SolverSolutions::default();
	let mut solver_stats = plugin::SolverStats::default();
	let mut nonce = 0;
	while solver_sols.num_sols == 0 {
		solver_sols = plugin::SolverSolutions::default();
		plugin_lib.run_solver(solver_ctx, gen.header.pre_pow(), nonce, 1, &mut solver_sols, &mut solver_stats);
		nonce += 1;
	}

	// set the PoW solution and make sure the block is mostly valid
	gen.header.pow.nonce = solver_sols.sols[0].nonce as u64;
	gen.header.pow.proof.nonces = solver_sols.sols[0].to_u64s();
	assert!(gen.header.pow.is_secondary(), "Not a secondary header");
	core::pow::verify_size(&gen.header).unwrap();
	gen.validate(&BlindingFactor::zero(), Arc::new(util::RwLock::new(LruVerifierCache::new()))).unwrap();

	// TODO check again the bitcoin block to make sure it's not been orphaned
	// TODO Commit genesis block info in git and tag
}

fn setup_chain(dir_name: &str, genesis: core::core::Block) -> chain::Chain {
	util::init_test_logger();
	let _ = fs::remove_dir_all(dir_name);
	let verifier_cache = Arc::new(util::RwLock::new(
		core::core::verifier_cache::LruVerifierCache::new(),
	));
	let db_env = Arc::new(store::new_env(dir_name.to_string()));
	chain::Chain::init(
		dir_name.to_string(),
		db_env,
		Arc::new(chain::types::NoopAdapter {}),
		genesis,
		core::pow::verify_size,
		verifier_cache,
		false,
	)
	.unwrap()
}

fn get_bchain_head() -> String {
	get_json(BCHAIN_INFO_URL)["hash"]
		.as_str()
		.unwrap()
		.to_string()
}

fn get_bcypher_head() -> String {
	get_json(BCYPHER_URL)["hash"].as_str().unwrap().to_string()
}

fn get_bchair_head() -> String {
	get_json(BCHAIR_URL)["data"][0]["hash"]
		.as_str()
		.unwrap()
		.to_string()
}

fn get_json(url: &str) -> serde_json::Value {
	let mut body = Vec::new();
	let mut easy = curl::easy::Easy::new();
	easy.url(url).unwrap();
	{
		let mut transfer = easy.transfer();
		transfer
			.write_function(|data| {
				body.extend_from_slice(data);
				Ok(data.len())
			})
			.unwrap();
		transfer.perform().unwrap();
	}
	serde_json::from_slice(&body).unwrap()
}
