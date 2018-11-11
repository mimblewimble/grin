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
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use util::{Mutex, RwLock};

use common::api;
use common::serde_json;
use store;
use util;
use util::secp::pedersen::Commitment;

use common::failure::ResultExt;

use chain::types::NoopAdapter;
use chain::Chain;
use core::core::verifier_cache::LruVerifierCache;
use core::core::Transaction;
use core::global::{set_mining_mode, ChainTypes};
use core::{pow, ser};
use keychain::Keychain;

use util::secp::pedersen;
use wallet::libtx::slate::Slate;
use wallet::libwallet;
use wallet::libwallet::types::*;

use common;

/// Messages to simulate wallet requests/responses
#[derive(Clone, Debug)]
pub struct WalletProxyMessage {
	/// sender ID
	pub sender_id: String,
	/// destination wallet (or server)
	pub dest: String,
	/// method (like a GET url)
	pub method: String,
	/// payload (json body)
	pub body: String,
}

/// communicates with a chain instance or other wallet
/// listener APIs via message queues
pub struct WalletProxy<C, K>
where
	C: WalletClient,
	K: Keychain,
{
	/// directory to create the chain in
	pub chain_dir: String,
	/// handle to chain itself
	pub chain: Arc<Chain>,
	/// list of interested wallets
	pub wallets: HashMap<
		String,
		(
			Sender<WalletProxyMessage>,
			Arc<Mutex<WalletInst<LocalWalletClient, K>>>,
		),
	>,
	/// simulate json send to another client
	/// address, method, payload (simulate HTTP request)
	pub tx: Sender<WalletProxyMessage>,
	/// simulate json receiving
	pub rx: Receiver<WalletProxyMessage>,
	/// queue control
	pub running: Arc<AtomicBool>,
	/// Phantom
	phantom_c: PhantomData<C>,
	/// Phantom
	phantom_k: PhantomData<K>,
}

impl<C, K> WalletProxy<C, K>
where
	C: WalletClient,
	K: Keychain,
{
	/// Create a new client that will communicate with the given grin node
	pub fn new(chain_dir: &str) -> Self {
		set_mining_mode(ChainTypes::AutomatedTesting);
		let genesis_block = pow::mine_genesis_block().unwrap();
		let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));
		let dir_name = format!("{}/.grin", chain_dir);
		let db_env = Arc::new(store::new_env(dir_name.to_string()));
		let c = Chain::init(
			dir_name.to_string(),
			db_env,
			Arc::new(NoopAdapter {}),
			genesis_block,
			pow::verify_size,
			verifier_cache,
			false,
		).unwrap();
		let (tx, rx) = channel();
		let retval = WalletProxy {
			chain_dir: chain_dir.to_owned(),
			chain: Arc::new(c),
			tx: tx,
			rx: rx,
			wallets: HashMap::new(),
			running: Arc::new(AtomicBool::new(false)),
			phantom_c: PhantomData,
			phantom_k: PhantomData,
		};
		retval
	}

	/// Add wallet with a given "address"
	pub fn add_wallet(
		&mut self,
		addr: &str,
		tx: Sender<WalletProxyMessage>,
		wallet: Arc<Mutex<WalletInst<LocalWalletClient, K>>>,
	) {
		self.wallets.insert(addr.to_owned(), (tx, wallet));
	}

	/// Run the incoming message queue and respond more or less
	/// synchronously
	pub fn run(&mut self) -> Result<(), libwallet::Error> {
		self.running.store(true, Ordering::Relaxed);
		loop {
			thread::sleep(Duration::from_millis(10));
			// read queue
			let m = self.rx.recv().unwrap();
			trace!("Wallet Client Proxy Received: {:?}", m);
			let resp = match m.method.as_ref() {
				"get_chain_height" => self.get_chain_height(m)?,
				"get_outputs_from_node" => self.get_outputs_from_node(m)?,
				"get_outputs_by_pmmr_index" => self.get_outputs_by_pmmr_index(m)?,
				"send_tx_slate" => self.send_tx_slate(m)?,
				"post_tx" => self.post_tx(m)?,
				_ => panic!("Unknown Wallet Proxy Message"),
			};

			self.respond(resp);
			if !self.running.load(Ordering::Relaxed) {
				return Ok(());
			}
		}
	}

	/// Return a message to a given wallet client
	fn respond(&mut self, m: WalletProxyMessage) {
		if let Some(s) = self.wallets.get_mut(&m.dest) {
			if let Err(e) = s.0.send(m.clone()) {
				panic!("Error sending response from proxy: {:?}, {}", m, e);
			}
		} else {
			panic!("Unknown wallet recipient for response message: {:?}", m);
		}
	}

	/// post transaction to the chain (and mine it, taking the reward)
	fn post_tx(&mut self, m: WalletProxyMessage) -> Result<WalletProxyMessage, libwallet::Error> {
		let dest_wallet = self.wallets.get_mut(&m.sender_id).unwrap().1.clone();
		let wrapper: TxWrapper = serde_json::from_str(&m.body).context(
			libwallet::ErrorKind::ClientCallback("Error parsing TxWrapper"),
		)?;

		let tx_bin = util::from_hex(wrapper.tx_hex).context(
			libwallet::ErrorKind::ClientCallback("Error parsing TxWrapper: tx_bin"),
		)?;

		let tx: Transaction = ser::deserialize(&mut &tx_bin[..]).context(
			libwallet::ErrorKind::ClientCallback("Error parsing TxWrapper: tx"),
		)?;

		common::award_block_to_wallet(&self.chain, vec![&tx], dest_wallet)?;

		Ok(WalletProxyMessage {
			sender_id: "node".to_owned(),
			dest: m.sender_id,
			method: m.method,
			body: "".to_owned(),
		})
	}

	/// send tx slate
	fn send_tx_slate(
		&mut self,
		m: WalletProxyMessage,
	) -> Result<WalletProxyMessage, libwallet::Error> {
		let dest_wallet = self.wallets.get_mut(&m.dest);
		if let None = dest_wallet {
			panic!("Unknown wallet destination for send_tx_slate: {:?}", m);
		}
		let w = dest_wallet.unwrap().1.clone();
		let mut slate = serde_json::from_str(&m.body).unwrap();
		libwallet::controller::foreign_single_use(w.clone(), |listener_api| {
			listener_api.receive_tx(&mut slate)?;
			Ok(())
		})?;
		Ok(WalletProxyMessage {
			sender_id: m.dest,
			dest: m.sender_id,
			method: m.method,
			body: serde_json::to_string(&slate).unwrap(),
		})
	}

	/// get chain height
	fn get_chain_height(
		&mut self,
		m: WalletProxyMessage,
	) -> Result<WalletProxyMessage, libwallet::Error> {
		Ok(WalletProxyMessage {
			sender_id: "node".to_owned(),
			dest: m.sender_id,
			method: m.method,
			body: format!("{}", self.chain.head().unwrap().height).to_owned(),
		})
	}

	/// get api outputs
	fn get_outputs_from_node(
		&mut self,
		m: WalletProxyMessage,
	) -> Result<WalletProxyMessage, libwallet::Error> {
		let split = m.body.split(",");
		//let mut api_outputs: HashMap<pedersen::Commitment, String> = HashMap::new();
		let mut outputs: Vec<api::Output> = vec![];
		for o in split {
			let o_str = String::from(o);
			if o_str.len() == 0 {
				continue;
			}
			let c = util::from_hex(o_str).unwrap();
			let commit = Commitment::from_vec(c);
			let out = common::get_output_local(&self.chain.clone(), &commit);
			if let Some(o) = out {
				outputs.push(o);
			}
		}
		Ok(WalletProxyMessage {
			sender_id: "node".to_owned(),
			dest: m.sender_id,
			method: m.method,
			body: serde_json::to_string(&outputs).unwrap(),
		})
	}

	/// get api outputs
	fn get_outputs_by_pmmr_index(
		&mut self,
		m: WalletProxyMessage,
	) -> Result<WalletProxyMessage, libwallet::Error> {
		let split = m.body.split(",").collect::<Vec<&str>>();
		let start_index = split[0].parse::<u64>().unwrap();
		let max = split[1].parse::<u64>().unwrap();
		let ol = common::get_outputs_by_pmmr_index_local(self.chain.clone(), start_index, max);
		Ok(WalletProxyMessage {
			sender_id: "node".to_owned(),
			dest: m.sender_id,
			method: m.method,
			body: serde_json::to_string(&ol).unwrap(),
		})
	}
}

#[derive(Clone)]
pub struct LocalWalletClient {
	/// wallet identifier for the proxy queue
	pub id: String,
	/// proxy's tx queue (receive messages from other wallets or node
	pub proxy_tx: Arc<Mutex<Sender<WalletProxyMessage>>>,
	/// my rx queue
	pub rx: Arc<Mutex<Receiver<WalletProxyMessage>>>,
	/// my tx queue
	pub tx: Arc<Mutex<Sender<WalletProxyMessage>>>,
}

impl LocalWalletClient {
	/// new
	pub fn new(id: &str, proxy_rx: Sender<WalletProxyMessage>) -> Self {
		let (tx, rx) = channel();
		LocalWalletClient {
			id: id.to_owned(),
			proxy_tx: Arc::new(Mutex::new(proxy_rx)),
			rx: Arc::new(Mutex::new(rx)),
			tx: Arc::new(Mutex::new(tx)),
		}
	}

	/// get an instance of the send queue for other senders
	pub fn get_send_instance(&self) -> Sender<WalletProxyMessage> {
		self.tx.lock().clone()
	}
}

impl WalletClient for LocalWalletClient {
	fn node_url(&self) -> &str {
		"node"
	}
	fn node_api_secret(&self) -> Option<String> {
		None
	}

	/// Call the wallet API to create a coinbase output for the given
	/// block_fees. Will retry based on default "retry forever with backoff"
	/// behavior.
	fn create_coinbase(
		&self,
		_dest: &str,
		_block_fees: &BlockFees,
	) -> Result<CbData, libwallet::Error> {
		unimplemented!();
	}

	/// Send the slate to a listening wallet instance
	fn send_tx_slate(&self, dest: &str, slate: &Slate) -> Result<Slate, libwallet::Error> {
		let m = WalletProxyMessage {
			sender_id: self.id.clone(),
			dest: dest.to_owned(),
			method: "send_tx_slate".to_owned(),
			body: serde_json::to_string(slate).unwrap(),
		};
		{
			let p = self.proxy_tx.lock();
			p.send(m)
				.context(libwallet::ErrorKind::ClientCallback("Send TX Slate"))?;
		}
		let r = self.rx.lock();
		let m = r.recv().unwrap();
		trace!("Received send_tx_slate response: {:?}", m.clone());
		Ok(
			serde_json::from_str(&m.body).context(libwallet::ErrorKind::ClientCallback(
				"Parsing send_tx_slate response",
			))?,
		)
	}

	/// Posts a transaction to a grin node
	/// In this case it will create a new block with award rewarded to
	fn post_tx(&self, tx: &TxWrapper, _fluff: bool) -> Result<(), libwallet::Error> {
		let m = WalletProxyMessage {
			sender_id: self.id.clone(),
			dest: self.node_url().to_owned(),
			method: "post_tx".to_owned(),
			body: serde_json::to_string(tx).unwrap(),
		};
		{
			let p = self.proxy_tx.lock();
			p.send(m)
				.context(libwallet::ErrorKind::ClientCallback("post_tx send"))?;
		}
		let r = self.rx.lock();
		let m = r.recv().unwrap();
		trace!("Received post_tx response: {:?}", m.clone());
		Ok(())
	}

	/// Return the chain tip from a given node
	fn get_chain_height(&self) -> Result<u64, libwallet::Error> {
		let m = WalletProxyMessage {
			sender_id: self.id.clone(),
			dest: self.node_url().to_owned(),
			method: "get_chain_height".to_owned(),
			body: "".to_owned(),
		};
		{
			let p = self.proxy_tx.lock();
			p.send(m).context(libwallet::ErrorKind::ClientCallback(
				"Get chain height send",
			))?;
		}
		let r = self.rx.lock();
		let m = r.recv().unwrap();
		trace!("Received get_chain_height response: {:?}", m.clone());
		Ok(m.body
			.parse::<u64>()
			.context(libwallet::ErrorKind::ClientCallback(
				"Parsing get_height response",
			))?)
	}

	/// Retrieve outputs from node
	fn get_outputs_from_node(
		&self,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<HashMap<pedersen::Commitment, (String, u64)>, libwallet::Error> {
		let query_params: Vec<String> = wallet_outputs
			.iter()
			.map(|commit| format!("{}", util::to_hex(commit.as_ref().to_vec())))
			.collect();
		let query_str = query_params.join(",");
		let m = WalletProxyMessage {
			sender_id: self.id.clone(),
			dest: self.node_url().to_owned(),
			method: "get_outputs_from_node".to_owned(),
			body: query_str,
		};
		{
			let p = self.proxy_tx.lock();
			p.send(m).context(libwallet::ErrorKind::ClientCallback(
				"Get outputs from node send",
			))?;
		}
		let r = self.rx.lock();
		let m = r.recv().unwrap();
		let outputs: Vec<api::Output> = serde_json::from_str(&m.body).unwrap();
		let mut api_outputs: HashMap<pedersen::Commitment, (String, u64)> = HashMap::new();
		for out in outputs {
			api_outputs.insert(
				out.commit.commit(),
				(util::to_hex(out.commit.to_vec()), out.height),
			);
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
			Vec<(pedersen::Commitment, pedersen::RangeProof, bool, u64)>,
		),
		libwallet::Error,
	> {
		// start index, max
		let query_str = format!("{},{}", start_height, max_outputs);
		let m = WalletProxyMessage {
			sender_id: self.id.clone(),
			dest: self.node_url().to_owned(),
			method: "get_outputs_by_pmmr_index".to_owned(),
			body: query_str,
		};
		{
			let p = self.proxy_tx.lock();
			p.send(m).context(libwallet::ErrorKind::ClientCallback(
				"Get outputs from node by PMMR index send",
			))?;
		}

		let r = self.rx.lock();
		let m = r.recv().unwrap();
		let o: api::OutputListing = serde_json::from_str(&m.body).unwrap();

		let mut api_outputs: Vec<(pedersen::Commitment, pedersen::RangeProof, bool, u64)> =
			Vec::new();

		for out in o.outputs {
			let is_coinbase = match out.output_type {
				api::OutputType::Coinbase => true,
				api::OutputType::Transaction => false,
			};
			api_outputs.push((
				out.commit,
				out.range_proof().unwrap(),
				is_coinbase,
				out.block_height.unwrap(),
			));
		}
		Ok((o.highest_index, o.last_retrieved_index, api_outputs))
	}
}
