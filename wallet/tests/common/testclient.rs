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

//! Test client that acts against a local instance of a node
//! so that wallet API can be fully exercised
//! Operates directly on a chain instance

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use store;
use util;

use chain::types::NoopAdapter;
use chain::Chain;
use core::global::{set_mining_mode, ChainTypes};
use core::pow;
use keychain::Keychain;

use util::secp::pedersen;
use wallet::libtx::slate::Slate;
use wallet::libwallet;
use wallet::libwallet::api::APIForeign;
use wallet::libwallet::types::*;
use wallet::FileWallet;

use common;

#[derive(Clone)]
pub struct TestWalletClient<'a, K>
where
	K: Keychain + 'a,
{
	/// dir to create chain in
	pub chain_dir: String,
	/// handle to chain itself
	pub chain: Arc<Chain>,
	/// set this to provide a recipient API for the tx_send function
	pub tx_recipient: Option<Arc<Mutex<Box<FileWallet<TestWalletClient<'a, K>, K>>>>>,
}

impl<'a, K> TestWalletClient<'a, K>
where
	K: Keychain + 'a,
{
	/// Create a new client that will communicate with the given grin node
	pub fn new(chain_dir: &str) -> Self {
		set_mining_mode(ChainTypes::AutomatedTesting);
		let genesis_block = pow::mine_genesis_block().unwrap();
		let dir_name = format!("{}", chain_dir);
		let db_env = Arc::new(store::new_env(dir_name.to_string()));
		let c = Chain::init(
			dir_name.to_string(),
			db_env,
			Arc::new(NoopAdapter {}),
			genesis_block,
			pow::verify_size,
		).unwrap();

		TestWalletClient {
			chain_dir: chain_dir.to_owned(),
			chain: Arc::new(c),
			tx_recipient: None,
		}
	}

	/// Set the wallet for the client send
	pub fn set_tx_recipient(
		&mut self,
		w: Arc<Mutex<Box<FileWallet<TestWalletClient<'a, K>, K>>>>,
	) -> Result<(), libwallet::Error> {
		self.tx_recipient = Some(w);
		Ok(())
	}
}

impl<'a, K> WalletClient for TestWalletClient<'a, K>
where
	K: Keychain + 'a,
{
	fn node_url(&self) -> &str {
		&self.chain_dir
	}

	/// Call the wallet API to create a coinbase output for the given
	/// block_fees. Will retry based on default "retry forever with backoff"
	/// behavior.
	fn create_coinbase(
		&self,
		dest: &str,
		block_fees: &BlockFees,
	) -> Result<CbData, libwallet::Error> {
		unimplemented!();
	}

	/// Send the slate to a listening wallet instance
	fn send_tx_slate(&self, dest: &str, slate: &Slate) -> Result<Slate, libwallet::Error> {
		let mut slate = slate.clone();
		libwallet::controller::foreign_single_use(
			self.tx_recipient.as_ref().unwrap().clone(),
			|listener_api| {
				listener_api.receive_tx(&mut slate);
				Ok(())
			},
		)?;
		Ok(slate.clone())
	}

	/// Posts a transaction to a grin node
	fn post_tx(&self, tx: &TxWrapper, fluff: bool) -> Result<(), libwallet::Error> {
		unimplemented!()
	}

	/// Return the chain tip from a given node
	fn get_chain_height(&self) -> Result<u64, libwallet::Error> {
		Ok(self.chain.head().unwrap().height)
	}

	/// Retrieve outputs from node
	fn get_outputs_from_node(
		&self,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<HashMap<pedersen::Commitment, String>, libwallet::Error> {
		let mut api_outputs: HashMap<pedersen::Commitment, String> = HashMap::new();
		for c in wallet_outputs.iter() {
			let out = common::get_output_local(&self.chain.clone(), c)?;
			api_outputs.insert(out.commit.commit(), util::to_hex(out.commit.to_vec()));
		}
		Ok(api_outputs)
	}

	fn get_outputs_by_pmmr_index(
		&self,
		start_height: u64,
		max_outputs: u64,
	) -> Result<
		(
			u64,
			u64,
			Vec<(pedersen::Commitment, pedersen::RangeProof, bool)>,
		),
		libwallet::Error,
	> {
		unimplemented!();
	}
}
