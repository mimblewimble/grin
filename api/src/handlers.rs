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

use std::io::Read;
use std::sync::{Arc, RwLock};
use std::thread;

use iron::prelude::*;
use iron::Handler;
use iron::status;
use urlencoded::UrlEncodedQuery;
use serde::Serialize;
use serde_json;

use chain;
use core::core::Transaction;
use core::core::hash::Hash;
use core::core::hash::Hashed;
use core::ser;
use pool;
use p2p;
use regex::Regex;
use rest::*;
use util::secp::pedersen::Commitment;
use types::*;
use util;
use util::LOGGER;

// RESTful index of available api endpoints
// GET /v1/
struct IndexHandler {
	list: Vec<String>,
}

impl IndexHandler {}

impl Handler for IndexHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		json_response_pretty(&self.list)
	}
}

// Supports retrieval of multiple outputs in a single request -
// GET /v1/chain/utxos/byids?id=xxx,yyy,zzz
// GET /v1/chain/utxos/byids?id=xxx&id=yyy&id=zzz
// GET /v1/chain/utxos/byheight?height=n
struct UtxoHandler {
	chain: Arc<chain::Chain>,
}

impl UtxoHandler {
	fn get_utxo(&self, id: &str, include_rp: bool, include_switch: bool) -> Result<Output, Error> {
		debug!(LOGGER, "getting utxo: {}", id);
		let c = util::from_hex(String::from(id))
			.map_err(|_| Error::Argument(format!("Not a valid commitment: {}", id)))?;
		let commit = Commitment::from_vec(c);

		let out = self.chain
			.get_unspent(&commit)
			.map_err(|_| Error::NotFound)?;

		let header = self.chain
			.get_block_header_by_output_commit(&commit)
			.map_err(|_| Error::NotFound)?;

		Ok(Output::from_output(
			&out,
			&header,
			include_rp,
			include_switch,
		))
	}

	fn utxos_by_ids(&self, req: &mut Request) -> Vec<Output> {
		let mut commitments: Vec<&str> = vec![];
		let mut rp = false;
		let mut switch = false;
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(ids) = params.get("id") {
				for id in ids {
					for id in id.split(",") {
						commitments.push(id.clone());
					}
				}
			}
			if let Some(_) = params.get("include_rp") {
				rp = true;
			}
			if let Some(_) = params.get("include_switch") {
				switch = true;
			}
		}
		let mut utxos: Vec<Output> = vec![];
		for commit in commitments {
			if let Ok(out) = self.get_utxo(commit, rp, switch) {
				utxos.push(out);
			}
		}
		utxos
	}

	fn utxos_at_height(&self, block_height: u64) -> BlockOutputs {
		let header = self.chain
			.clone()
			.get_header_by_height(block_height)
			.unwrap();
		let block = self.chain.clone().get_block(&header.hash()).unwrap();
		let outputs = block
			.outputs
			.iter()
			.filter(|c| self.chain.is_unspent(&c.commit).unwrap())
			.map(|k| OutputSwitch::from_output(k, &header))
			.collect();
		BlockOutputs {
			header: BlockHeaderInfo::from_header(&header),
			outputs: outputs,
		}
	}

	// returns utxos for a specified range of blocks
	fn utxo_block_batch(&self, req: &mut Request) -> Vec<BlockOutputs> {
		let mut start_height = 1;
		let mut end_height = 1;
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(heights) = params.get("start_height") {
				for height in heights {
					start_height = height.parse().unwrap();
				}
			}
			if let Some(heights) = params.get("end_height") {
				for height in heights {
					end_height = height.parse().unwrap();
				}
			}
		}
		let mut return_vec = vec![];
		for i in start_height..end_height + 1 {
			return_vec.push(self.utxos_at_height(i));
		}
		return_vec
	}
}

impl Handler for UtxoHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let url = req.url.clone();
		let mut path_elems = url.path();
		if *path_elems.last().unwrap() == "" {
			path_elems.pop();
		}
		match *path_elems.last().unwrap() {
			"byids" => json_response(&self.utxos_by_ids(req)),
			"byheight" => json_response(&self.utxo_block_batch(req)),
			_ => Ok(Response::with((status::BadRequest, ""))),
		}
	}
}

// Sum tree handler. Retrieve the roots:
// GET /v1/sumtrees/roots
//
// Last inserted nodes::
// GET /v1/sumtrees/lastutxos (gets last 10)
// GET /v1/sumtrees/lastutxos?n=5
// GET /v1/sumtrees/lastrangeproofs
// GET /v1/sumtrees/lastkernels
struct SumTreeHandler {
	chain: Arc<chain::Chain>,
}

impl SumTreeHandler {
	// gets roots
	fn get_roots(&self) -> SumTrees {
		SumTrees::from_head(self.chain.clone())
	}

	// gets last n utxos inserted in to the tree
	fn get_last_n_utxo(&self, distance: u64) -> Vec<SumTreeNode> {
		SumTreeNode::get_last_n_utxo(self.chain.clone(), distance)
	}

	// gets last n utxos inserted in to the tree
	fn get_last_n_rangeproof(&self, distance: u64) -> Vec<SumTreeNode> {
		SumTreeNode::get_last_n_rangeproof(self.chain.clone(), distance)
	}

	// gets last n utxos inserted in to the tree
	fn get_last_n_kernel(&self, distance: u64) -> Vec<SumTreeNode> {
		SumTreeNode::get_last_n_kernel(self.chain.clone(), distance)
	}
}

impl Handler for SumTreeHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let url = req.url.clone();
		let mut path_elems = url.path();
		if *path_elems.last().unwrap() == "" {
			path_elems.pop();
		}
		// TODO: probably need to set a reasonable max limit here
		let mut last_n = 10;
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(nums) = params.get("n") {
				for num in nums {
					if let Ok(n) = str::parse(num) {
						last_n = n;
					}
				}
			}
		}
		match *path_elems.last().unwrap() {
			"roots" => json_response_pretty(&self.get_roots()),
			"lastutxos" => json_response_pretty(&self.get_last_n_utxo(last_n)),
			"lastrangeproofs" => json_response_pretty(&self.get_last_n_rangeproof(last_n)),
			"lastkernels" => json_response_pretty(&self.get_last_n_kernel(last_n)),
			_ => Ok(Response::with((status::BadRequest, ""))),
		}
	}
}

pub struct PeersAllHandler {
	pub peers: p2p::Peers,
}

impl Handler for PeersAllHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		let peers = &self.peers.all_peers();
		json_response_pretty(&peers)
	}
}

pub struct PeersConnectedHandler {
	pub peers: p2p::Peers,
}

impl Handler for PeersConnectedHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		let mut peers = vec![];
		for p in &self.peers.connected_peers() {
			let p = p.read().unwrap();
			let peer_info = p.info.clone();
			peers.push(peer_info);
		}
		json_response(&peers)
	}
}

/// Peer operations
/// POST /v1/peers/10.12.12.13/ban
/// POST /v1/peers/10.12.12.13/unban
pub struct PeerPostHandler {
	pub peers: p2p::Peers,
}

impl Handler for PeerPostHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let url = req.url.clone();
		let mut path_elems = url.path();
		if *path_elems.last().unwrap() == "" {
			path_elems.pop();
		}
		match *path_elems.last().unwrap() {
			"ban" => {
				path_elems.pop();
				if let Ok(addr) = path_elems.last().unwrap().parse() {
					self.peers.ban_peer(&addr);
					Ok(Response::with((status::Ok, "")))
				} else {
					Ok(Response::with((status::BadRequest, "")))
				}
			}
			"unban" => {
				path_elems.pop();
				if let Ok(addr) = path_elems.last().unwrap().parse() {
					self.peers.unban_peer(&addr);
					Ok(Response::with((status::Ok, "")))
				} else {
					Ok(Response::with((status::BadRequest, "")))
				}
			}
			_ => Ok(Response::with((status::BadRequest, ""))),
		}
	}
}

/// Get details about a given peer
pub struct PeerGetHandler {
	pub peers: p2p::Peers,
}

impl Handler for PeerGetHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let url = req.url.clone();
		let mut path_elems = url.path();
		if *path_elems.last().unwrap() == "" {
			path_elems.pop();
		}
		if let Ok(addr) = path_elems.last().unwrap().parse() {
			match self.peers.get_peer(addr) {
				Ok(peer) => json_response(&peer),
				Err(_) => Ok(Response::with((status::BadRequest, ""))),
			}
		} else {
			Ok(Response::with((status::BadRequest, "")))
		}
	}
}

// Status handler. Post a summary of the server status
// GET /v1/status
pub struct StatusHandler {
	pub chain: Arc<chain::Chain>,
	pub peers: p2p::Peers,
}

impl StatusHandler {
	fn get_status(&self) -> Status {
		Status::from_tip_and_peers(self.chain.head().unwrap(), self.peers.peer_count())
	}
}

impl Handler for StatusHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		json_response(&self.get_status())
	}
}

// Chain handler. Get the head details.
// GET /v1/chain
pub struct ChainHandler {
	pub chain: Arc<chain::Chain>,
}

impl ChainHandler {
	fn get_tip(&self) -> Tip {
		Tip::from_tip(self.chain.head().unwrap())
	}
}

impl Handler for ChainHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		json_response(&self.get_tip())
	}
}

// Gets block details given either a hash or height.
// GET /v1/block/<hash>
// GET /v1/block/<height>
pub struct BlockHandler {
	pub chain: Arc<chain::Chain>,
}

impl BlockHandler {
	fn get_block(&self, h: &Hash) -> Result<BlockPrintable, Error> {
		let block = self.chain
			.clone()
			.get_block(h)
			.map_err(|_| Error::NotFound)?;
		Ok(BlockPrintable::from_block(&block))
	}

	// Try to decode the string as a height or a hash.
	fn parse_input(&self, input: String) -> Result<Hash, Error> {
		if let Ok(height) = input.parse() {
			match self.chain.clone().get_header_by_height(height) {
				Ok(header) => return Ok(header.hash()),
				Err(_) => return Err(Error::NotFound),
			}
		}
		lazy_static! {
			static ref RE: Regex = Regex::new(r"[0-9a-fA-F]{64}").unwrap();
		}
		if !RE.is_match(&input) {
			return Err(Error::Argument(String::from("Not a valid hash or height.")));
		}
		let vec = util::from_hex(input).unwrap();
		Ok(Hash::from_vec(vec))
	}
}

impl Handler for BlockHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let url = req.url.clone();
		let mut path_elems = url.path();
		if *path_elems.last().unwrap() == "" {
			path_elems.pop();
		}
		let el = *path_elems.last().unwrap();
		let h = try!(self.parse_input(el.to_string()));
		let b = try!(self.get_block(&h));
		json_response(&b)
	}
}

// Get basic information about the transaction pool.
struct PoolInfoHandler<T> {
	tx_pool: Arc<RwLock<pool::TransactionPool<T>>>,
}

impl<T> Handler for PoolInfoHandler<T>
where
	T: pool::BlockChain + Send + Sync + 'static,
{
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		let pool = self.tx_pool.read().unwrap();
		json_response(&PoolInfo {
			pool_size: pool.pool_size(),
			orphans_size: pool.orphans_size(),
			total_size: pool.total_size(),
		})
	}
}

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
struct TxWrapper {
	tx_hex: String,
}

// Push new transactions to our transaction pool, that should broadcast it
// to the network if valid.
struct PoolPushHandler<T> {
	tx_pool: Arc<RwLock<pool::TransactionPool<T>>>,
}

impl<T> Handler for PoolPushHandler<T>
where
	T: pool::BlockChain + Send + Sync + 'static,
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let wrapper: TxWrapper = serde_json::from_reader(req.body.by_ref())
			.map_err(|e| IronError::new(e, status::BadRequest))?;

		let tx_bin = util::from_hex(wrapper.tx_hex)
			.map_err(|_| Error::Argument(format!("Invalid hex in transaction wrapper.")))?;

		let tx: Transaction = ser::deserialize(&mut &tx_bin[..]).map_err(|_| {
			Error::Argument("Could not deserialize transaction, invalid format.".to_string())
		})?;

		let source = pool::TxSource {
			debug_name: "push-api".to_string(),
			identifier: "?.?.?.?".to_string(),
		};
		info!(
			LOGGER,
			"Pushing transaction with {} inputs and {} outputs to pool.",
			tx.inputs.len(),
			tx.outputs.len()
		);

		let res = self.tx_pool.write().unwrap().add_to_memory_pool(source, tx);

		match res {
			Ok(()) => Ok(Response::with(status::Ok)),
			Err(e) => Err(IronError::from(Error::Argument(format!("{:?}", e)))),
		}
	}
}

// Utility to serialize a struct into JSON and produce a sensible IronResult
// out of it.
fn json_response<T>(s: &T) -> IronResult<Response>
where
	T: Serialize,
{
	match serde_json::to_string(s) {
		Ok(json) => Ok(Response::with((status::Ok, json))),
		Err(_) => Ok(Response::with((status::InternalServerError, ""))),
	}
}

// pretty-printed version of above
fn json_response_pretty<T>(s: &T) -> IronResult<Response>
where
	T: Serialize,
{
	match serde_json::to_string_pretty(s) {
		Ok(json) => Ok(Response::with((status::Ok, json))),
		Err(_) => Ok(Response::with((status::InternalServerError, ""))),
	}
}
/// Start all server HTTP handlers. Register all of them with Iron
/// and runs the corresponding HTTP server.
pub fn start_rest_apis<T>(
	addr: String,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<T>>>,
	peers: p2p::Peers,
) where
	T: pool::BlockChain + Send + Sync + 'static,
{
	let _ = thread::Builder::new()
		.name("apis".to_string())
		.spawn(move || {
			// build handlers and register them under the appropriate endpoint
			let utxo_handler = UtxoHandler {
				chain: chain.clone(),
			};
			let block_handler = BlockHandler {
				chain: chain.clone(),
			};
			let chain_tip_handler = ChainHandler {
				chain: chain.clone(),
			};
			let status_handler = StatusHandler {
				chain: chain.clone(),
				peers: peers.clone(),
			};
			let sumtree_handler = SumTreeHandler {
				chain: chain.clone(),
			};
			let pool_info_handler = PoolInfoHandler {
				tx_pool: tx_pool.clone(),
			};
			let pool_push_handler = PoolPushHandler {
				tx_pool: tx_pool.clone(),
			};
			let peers_all_handler = PeersAllHandler {
				peers: peers.clone(),
			};
			let peers_connected_handler = PeersConnectedHandler {
				peers: peers.clone(),
			};
			let peer_post_handler = PeerPostHandler {
				peers: peers.clone(),
			};
			let peer_get_handler = PeerGetHandler {
				peers: peers.clone(),
			};

			let route_list = vec![
				"get blocks".to_string(),
				"get chain".to_string(),
				"get chain/utxos".to_string(),
				"get status".to_string(),
				"get sumtrees/roots".to_string(),
				"get sumtrees/lastutxos?n=10".to_string(),
				"get sumtrees/lastrangeproofs".to_string(),
				"get sumtrees/lastkernels".to_string(),
				"get pool".to_string(),
				"post pool/push".to_string(),
				"post peers/a.b.c.d:p/ban".to_string(),
				"post peers/a.b.c.d:p/unban".to_string(),
				"get peers/all".to_string(),
				"get peers/connected".to_string(),
				"get peers/a.b.c.d".to_string(),
			];
			// We allow manually banning, like this:
			// curl -v -X POST http://127.0.0.1:13413/v1/peers/88.99.251.87:13414/ban
			let index_handler = IndexHandler { list: route_list };
			let router = router!(
				index: get "/" => index_handler,
				blocks: get "/blocks/*" => block_handler,
				chain_tip: get "/chain" => chain_tip_handler,
				chain_utxos: get "/chain/utxos/*" => utxo_handler,
				status: get "/status" => status_handler,
				sumtree_roots: get "/sumtrees/*" => sumtree_handler,
				pool_info: get "/pool" => pool_info_handler,
				pool_push: post "/pool/push" => pool_push_handler,
				peers_all: get "/peers/all" => peers_all_handler,
				peers_connected: get "/peers/connected" => peers_connected_handler,
				peer: post "/peers/*" => peer_post_handler,
				peer: get "/peers/*" => peer_get_handler
			);

			let mut apis = ApiServer::new("/v1".to_string());
			apis.register_handler(router);

			info!(LOGGER, "Starting HTTP API server at {}.", addr);
			apis.start(&addr[..]).unwrap_or_else(|e| {
				error!(LOGGER, "Failed to start API HTTP server: {}.", e);
			});
		});
}
