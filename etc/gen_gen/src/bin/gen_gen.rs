// Copyright 2021 The Grin Developers
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

use std::io::{BufRead, Write};
use std::sync::Arc;
use std::{fs, io, path};

use chrono::prelude::Utc;
use chrono::{Datelike, Duration, Timelike};
use curl;
use rpassword;
use serde_json;

use cuckoo_miner as cuckoo;
use grin_chain as chain;
use grin_core as core;
use grin_miner_plugin as plugin;
use grin_store as store;
use grin_util::{self as util, ToHex};
use grin_wallet as wallet;

use grin_core::core::hash::Hashed;
use grin_core::core::verifier_cache::LruVerifierCache;
use grin_keychain::{BlindingFactor, ExtKeychain, Keychain};

static BCHAIN_INFO_URL: &str = "https://blockchain.info/latestblock";
static BCYPHER_URL: &str = "https://api.blockcypher.com/v1/btc/main";
static BCHAIR_URL: &str = "https://api.blockchair.com/bitcoin/blocks?limit=2";

static GENESIS_RS_PATH: &str = "../../core/src/genesis.rs";
static PLUGIN_PATH: &str = "./cuckaroo_mean_cuda_29.cuckooplugin";
static WALLET_SEED_PATH: &str = "./wallet.seed";

fn main() {
	if !path::Path::new(GENESIS_RS_PATH).exists() {
		panic!(
			"File {} not found, make sure you're running this from the gen_gen directory",
			GENESIS_RS_PATH
		);
	}
	if !path::Path::new(PLUGIN_PATH).exists() {
		panic!(
			"File {} not found, make sure you're running this from the gen_gen directory",
			PLUGIN_PATH
		);
	}
	if !path::Path::new(WALLET_SEED_PATH).exists() {
		panic!(
			"File {} not found, make sure you're running this from the gen_gen directory",
			WALLET_SEED_PATH
		);
	}

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

	// build the basic parts of the genesis block header
	let mut gen = core::genesis::genesis_main();

	// build the wallet seed and derive a coinbase from local wallet.seed
	let seed = wallet::WalletSeed::from_file(
		&wallet::WalletConfig::default(),
		&rpassword::prompt_password_stdout("Password: ").unwrap(),
	)
	.unwrap();
	let keychain: ExtKeychain = seed.derive_keychain(false).unwrap();
	let key_id = ExtKeychain::derive_key_id(3, 1, 0, 0, 0);
	let reward = core::libtx::reward::output(&keychain, &key_id, 0).unwrap();
	gen = gen.with_reward(reward.0, reward.1);

	{
		// setup a tmp chain to set block header roots
		core::global::set_local_chain_type(core::global::ChainTypes::UserTesting);
		let tmp_chain = setup_chain(".grin.tmp", core::pow::mine_genesis_block().unwrap());
		tmp_chain.set_txhashset_roots(&mut gen).unwrap();
	}

	// sets the timestamp and prev_root from the bitcoin block (needs to be
	// after set_txhashset roots to not get overwritten)
	gen.header.timestamp = Utc::now() + Duration::minutes(45);
	gen.header.prev_root = core::core::hash::Hash::from_hex(&h1).unwrap();

	// mine a Cuckaroo29 block
	core::global::set_local_chain_type(core::global::ChainTypes::Mainnet);
	let plugin_lib = cuckoo::PluginLibrary::new(PLUGIN_PATH).unwrap();
	let mut params = plugin_lib.get_default_params();
	params.mutate_nonce = false;
	let solver_ctx = plugin_lib.create_solver_ctx(&mut params);

	let mut solver_sols = plugin::SolverSolutions::default();
	let mut solver_stats = plugin::SolverStats::default();
	let mut nonce = 0;
	while solver_sols.num_sols == 0 {
		solver_sols = plugin::SolverSolutions::default();
		gen.header.pow.nonce = nonce;
		let _ = plugin_lib.run_solver(
			solver_ctx,
			gen.header.pre_pow(),
			nonce,
			1,
			&mut solver_sols,
			&mut solver_stats,
		);
		if solver_stats.has_errored {
			println!(
				"Plugin {} has errored, device: {}. Reason: {}",
				solver_stats.get_plugin_name(),
				solver_stats.get_device_name(),
				solver_stats.get_error_reason(),
			);
			return;
		}

		nonce += 1;
	}

	// Set the PoW solution and make sure the block is mostly valid
	gen.header.pow.proof.nonces = solver_sols.sols[0].to_u64s();
	assert!(gen.header.pow.is_secondary(), "Not a secondary header");
	println!("Built genesis:\n{:?}", gen);
	core::pow::verify_size(&gen.header).unwrap();
	gen.validate(
		&BlindingFactor::zero(),
		Arc::new(util::RwLock::new(LruVerifierCache::new())),
	)
	.unwrap();

	println!("\nFinal genesis cyclehash: {}", gen.hash().to_hex());
	let gen_bin = core::ser::ser_vec(&gen).unwrap();
	println!("Final genesis full hash: {}\n", gen_bin.hash().to_hex());

	update_genesis_rs(&gen);
	println!("genesis.rs has been updated, check it and run mainnet_genesis_hash test");
	println!("also check bitcoin block {} hasn't been orphaned.", h1);
	println!("All done!");
}

fn update_genesis_rs(gen: &core::core::Block) {
	// set the replacement patterns
	let mut replacements = vec![];
	replacements.push((
		"timestamp".to_string(),
		format!(
			"Utc.ymd({}, {}, {}).and_hms({}, {}, {})",
			gen.header.timestamp.date().year(),
			gen.header.timestamp.date().month(),
			gen.header.timestamp.date().day(),
			gen.header.timestamp.time().hour(),
			gen.header.timestamp.time().minute(),
			gen.header.timestamp.time().second(),
		),
	));
	replacements.push((
		"prev_root".to_string(),
		format!(
			"Hash::from_hex(\"{}\").unwrap()",
			gen.header.prev_root.to_hex()
		),
	));
	replacements.push((
		"output_root".to_string(),
		format!(
			"Hash::from_hex(\"{}\").unwrap()",
			gen.header.output_root.to_hex()
		),
	));
	replacements.push((
		"range_proof_root".to_string(),
		format!(
			"Hash::from_hex(\"{}\").unwrap()",
			gen.header.range_proof_root.to_hex()
		),
	));
	replacements.push((
		"kernel_root".to_string(),
		format!(
			"Hash::from_hex(\"{}\").unwrap()",
			gen.header.kernel_root.to_hex()
		),
	));
	replacements.push((
		"total_kernel_offset".to_string(),
		format!(
			"BlindingFactor::from_hex(\"{}\").unwrap()",
			gen.header.total_kernel_offset.to_hex()
		),
	));
	replacements.push(("nonce".to_string(), format!("{}", gen.header.pow.nonce)));
	replacements.push((
		"nonces".to_string(),
		format!("vec!{:?}", gen.header.pow.proof.nonces),
	));
	replacements.push((
		"excess".to_string(),
		format!(
			"Commitment::from_vec(util::from_hex({:x?}.to_string()).unwrap())",
			gen.kernels()[0].excess.to_hex()
		),
	));
	replacements.push((
		"excess_sig".to_string(),
		format!(
			"Signature::from_raw_data(&{:?}).unwrap()",
			gen.kernels()[0].excess_sig.to_raw_data().to_vec(),
		),
	));
	replacements.push((
		"commit".to_string(),
		format!(
			"Commitment::from_vec(util::from_hex({:x?}.to_string()).unwrap())",
			util::to_hex(gen.outputs()[0].commitment().0.to_vec())
		),
	));
	replacements.push((
		"proof".to_string(),
		format!("{:?}", gen.outputs()[0].proof.bytes().to_vec()),
	));

	// check each possible replacement in the file, remove the replacement from
	// the list when found to avoid double replacements
	let mut replaced = String::new();
	{
		let genesis_rs = fs::File::open(GENESIS_RS_PATH).unwrap();
		let reader = io::BufReader::new(&genesis_rs);
		for rline in reader.lines() {
			let line = rline.unwrap();
			let mut has_replaced = false;
			if line.contains("REPLACE") {
				for (pos, replacement) in replacements.iter().enumerate() {
					if line.contains(&replacement.0) {
						replaced.push_str(&format!("{}: {},\n", replacement.0, replacement.1));
						replacements.remove(pos);
						has_replaced = true;
						break;
					}
				}
			}
			if !has_replaced {
				replaced.push_str(&format!("{}\n", line));
			}
		}
	}
	let mut genesis_rs = fs::File::create(GENESIS_RS_PATH).unwrap();
	genesis_rs.write_all(replaced.as_bytes()).unwrap();
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
		Arc::new(util::StopState::new()),
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
