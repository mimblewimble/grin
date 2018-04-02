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

use std::io::Read;
use std::sync::{Arc, RwLock, Weak};
use std::thread;

use iron::prelude::*;
use iron::Handler;
use iron::status;
use urlencoded::UrlEncodedQuery;
use serde::Serialize;
use serde_json;

use chain;
use core::core::{OutputFeatures, OutputIdentifier, Transaction};
use core::core::hash::{Hash, Hashed};
use core::ser;
use pool;
use p2p;
use regex::Regex;
use rest::*;
use util::secp::pedersen::Commitment;
use types::*;
use util;
use util::LOGGER;

// All handlers use `Weak` references instead of `Arc` to avoid cycles that
// can never be destroyed. These 2 functions are simple helpers to reduce the
// boilerplate of dealing with `Weak`.
fn w<T>(weak: &Weak<T>) -> Arc<T> {
	weak.upgrade().unwrap()
}

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
// GET /v1/chain/outputs/byids?id=xxx,yyy,zzz
// GET /v1/chain/outputs/byids?id=xxx&id=yyy&id=zzz
// GET /v1/chain/outputs/byheight?start_height=101&end_height=200
struct OutputHandler {
	chain: Weak<chain::Chain>,
}

impl OutputHandler {
	fn get_output(&self, id: &str) -> Result<Output, Error> {
		let c = util::from_hex(String::from(id))
			.map_err(|_| Error::Argument(format!("Not a valid commitment: {}", id)))?;
		let commit = Commitment::from_vec(c);

		// We need the features here to be able to generate the necessary hash
		// to compare against the hash in the output MMR.
		// For now we can just try both (but this probably needs to be part of the api
		// params)
		let outputs = [
			OutputIdentifier::new(OutputFeatures::DEFAULT_OUTPUT, &commit),
			OutputIdentifier::new(OutputFeatures::COINBASE_OUTPUT, &commit),
		];

		for x in outputs.iter() {
			if let Ok(_) = w(&self.chain).is_unspent(&x) {
				return Ok(Output::new(&commit));
			}
		}
		Err(Error::NotFound)
	}

	fn outputs_by_ids(&self, req: &mut Request) -> Vec<Output> {
		let mut commitments: Vec<&str> = vec![];
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(ids) = params.get("id") {
				for id in ids {
					for id in id.split(",") {
						commitments.push(id.clone());
					}
				}
			}
		}

		debug!(LOGGER, "outputs_by_ids: {:?}", commitments);

		let mut outputs: Vec<Output> = vec![];
		for x in commitments {
			if let Ok(output) = self.get_output(x) {
				outputs.push(output);
			}
		}
		outputs
	}

	fn outputs_at_height(
		&self,
		block_height: u64,
		commitments: Vec<Commitment>,
		include_proof: bool,
	) -> BlockOutputs {
		let header = w(&self.chain).get_header_by_height(block_height).unwrap();

		// TODO - possible to compact away blocks we care about
		// in the period between accepting the block and refreshing the wallet
		if let Ok(block) = w(&self.chain).get_block(&header.hash()) {
			let outputs = block
				.outputs
				.iter()
				.filter(|output| commitments.is_empty() || commitments.contains(&output.commit))
				.map(|output| {
					OutputPrintable::from_output(output, w(&self.chain), &header, include_proof)
				})
				.collect();

			BlockOutputs {
				header: BlockHeaderInfo::from_header(&header),
				outputs: outputs,
			}
		} else {
			debug!(
				LOGGER,
				"could not find block {:?} at height {}, maybe compacted?",
				&header.hash(),
				block_height,
			);

			BlockOutputs {
				header: BlockHeaderInfo::from_header(&header),
				outputs: vec![],
			}
		}
	}

	// returns outputs for a specified range of blocks
	fn outputs_block_batch(&self, req: &mut Request) -> Vec<BlockOutputs> {
		let mut commitments: Vec<Commitment> = vec![];
		let mut start_height = 1;
		let mut end_height = 1;
		let mut include_rp = false;

		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(ids) = params.get("id") {
				for id in ids {
					for id in id.split(",") {
						if let Ok(x) = util::from_hex(String::from(id)) {
							commitments.push(Commitment::from_vec(x));
						}
					}
				}
			}
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
			if let Some(_) = params.get("include_rp") {
				include_rp = true;
			}
		}

		debug!(
			LOGGER,
			"outputs_block_batch: {}-{}, {:?}, {:?}",
			start_height,
			end_height,
			commitments,
			include_rp,
		);

		let mut return_vec = vec![];
		for i in start_height..end_height + 1 {
			let res = self.outputs_at_height(i, commitments.clone(), include_rp);
			if res.outputs.len() > 0 {
				return_vec.push(res);
			}
		}

		return_vec
	}
}

impl Handler for OutputHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let url = req.url.clone();
		let mut path_elems = url.path();
		if *path_elems.last().unwrap() == "" {
			path_elems.pop();
		}
		match *path_elems.last().unwrap() {
			"byids" => json_response(&self.outputs_by_ids(req)),
			"byheight" => json_response(&self.outputs_block_batch(req)),
			_ => Ok(Response::with((status::BadRequest, ""))),
		}
	}
}

// Sum tree handler. Retrieve the roots:
// GET /v1/txhashset/roots
//
// Last inserted nodes::
// GET /v1/txhashset/lastoutputs (gets last 10)
// GET /v1/txhashset/lastoutputs?n=5
// GET /v1/txhashset/lastrangeproofs
// GET /v1/txhashset/lastkernels
struct TxHashSetHandler {
	chain: Weak<chain::Chain>,
}

impl TxHashSetHandler {
	// gets roots
	fn get_roots(&self) -> TxHashSet {
		TxHashSet::from_head(w(&self.chain))
	}

	// gets last n outputs inserted in to the tree
	fn get_last_n_output(&self, distance: u64) -> Vec<TxHashSetNode> {
		TxHashSetNode::get_last_n_output(w(&self.chain), distance)
	}

	// gets last n outputs inserted in to the tree
	fn get_last_n_rangeproof(&self, distance: u64) -> Vec<TxHashSetNode> {
		TxHashSetNode::get_last_n_rangeproof(w(&self.chain), distance)
	}

	// gets last n outputs inserted in to the tree
	fn get_last_n_kernel(&self, distance: u64) -> Vec<TxHashSetNode> {
		TxHashSetNode::get_last_n_kernel(w(&self.chain), distance)
	}
}

impl Handler for TxHashSetHandler {
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
			"lastoutputs" => json_response_pretty(&self.get_last_n_output(last_n)),
			"lastrangeproofs" => json_response_pretty(&self.get_last_n_rangeproof(last_n)),
			"lastkernels" => json_response_pretty(&self.get_last_n_kernel(last_n)),
			_ => Ok(Response::with((status::BadRequest, ""))),
		}
	}
}

pub struct PeersAllHandler {
	pub peers: Weak<p2p::Peers>,
}

impl Handler for PeersAllHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		let peers = &w(&self.peers).all_peers();
		json_response_pretty(&peers)
	}
}

pub struct PeersConnectedHandler {
	pub peers: Weak<p2p::Peers>,
}

impl Handler for PeersConnectedHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		let mut peers = vec![];
		for p in &w(&self.peers).connected_peers() {
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
	pub peers: Weak<p2p::Peers>,
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
					w(&self.peers).ban_peer(&addr);
					Ok(Response::with((status::Ok, "")))
				} else {
					Ok(Response::with((status::BadRequest, "")))
				}
			}
			"unban" => {
				path_elems.pop();
				if let Ok(addr) = path_elems.last().unwrap().parse() {
					w(&self.peers).unban_peer(&addr);
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
	pub peers: Weak<p2p::Peers>,
}

impl Handler for PeerGetHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let url = req.url.clone();
		let mut path_elems = url.path();
		if *path_elems.last().unwrap() == "" {
			path_elems.pop();
		}
		if let Ok(addr) = path_elems.last().unwrap().parse() {
			match w(&self.peers).get_peer(addr) {
				Ok(peer) => json_response(&peer),
				Err(_) => Ok(Response::with((status::BadRequest, ""))),
			}
		} else {
			Ok(Response::with((status::BadRequest, "")))
		}
	}
}

/// Status handler. Post a summary of the server status
/// GET /v1/status
pub struct StatusHandler {
	pub chain: Weak<chain::Chain>,
	pub peers: Weak<p2p::Peers>,
}

impl StatusHandler {
	fn get_status(&self) -> Status {
		Status::from_tip_and_peers(w(&self.chain).head().unwrap(), w(&self.peers).peer_count())
	}
}

impl Handler for StatusHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		json_response(&self.get_status())
	}
}

/// Chain handler. Get the head details.
/// GET /v1/chain
pub struct ChainHandler {
	pub chain: Weak<chain::Chain>,
}

impl ChainHandler {
	fn get_tip(&self) -> Tip {
		Tip::from_tip(w(&self.chain).head().unwrap())
	}
}

impl Handler for ChainHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		json_response(&self.get_tip())
	}
}

/// Chain validation handler.
/// GET /v1/chain/validate
pub struct ChainValidationHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for ChainValidationHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		// TODO - read skip_rproofs from query params
		w(&self.chain).validate(true).unwrap();
		Ok(Response::with((status::Ok, "{}")))
	}
}

/// Temporary - fix header by height index.
/// POST /v1/chain/height-index
pub struct HeaderByHeightHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for HeaderByHeightHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		match w(&self.chain).rebuild_header_by_height() {
			Ok(_) => Ok(Response::with((status::Ok, ""))),
			Err(_) => Ok(Response::with((status::InternalServerError, ""))),
		}
	}
}

/// Chain compaction handler. Trigger a compaction of the chain state to regain
/// storage space.
/// GET /v1/chain/compact
pub struct ChainCompactHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for ChainCompactHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		w(&self.chain).compact().unwrap();
		Ok(Response::with((status::Ok, "{}")))
	}
}

/// Gets block details given either a hash or height.
/// GET /v1/blocks/<hash>
/// GET /v1/blocks/<height>
///
/// Optionally return results as "compact blocks" by passing "?compact" query param
/// GET /v1/blocks/<hash>?compact
///
pub struct BlockHandler {
	pub chain: Weak<chain::Chain>,
}

impl BlockHandler {
	fn get_block(&self, h: &Hash) -> Result<BlockPrintable, Error> {
		let block = w(&self.chain).get_block(h).map_err(|_| Error::NotFound)?;
		Ok(BlockPrintable::from_block(&block, w(&self.chain), false))
	}

	fn get_compact_block(&self, h: &Hash) -> Result<CompactBlockPrintable, Error> {
		let block = w(&self.chain).get_block(h).map_err(|_| Error::NotFound)?;
		Ok(CompactBlockPrintable::from_compact_block(
			&block.as_compact_block(),
			w(&self.chain),
		))
	}

	// Try to decode the string as a height or a hash.
	fn parse_input(&self, input: String) -> Result<Hash, Error> {
		if let Ok(height) = input.parse() {
			match w(&self.chain).get_header_by_height(height) {
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

		let mut compact = false;
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(_) = params.get("compact") {
				compact = true;
			}
		}

		if compact {
			let b = try!(self.get_compact_block(&h));
			json_response(&b)
		} else {
			let b = try!(self.get_block(&h));
			json_response(&b)
		}
	}
}

// Get basic information about the transaction pool.
struct PoolInfoHandler<T> {
	tx_pool: Weak<RwLock<pool::TransactionPool<T>>>,
}

impl<T> Handler for PoolInfoHandler<T>
where
	T: pool::BlockChain + Send + Sync + 'static,
{
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		let pool_arc = w(&self.tx_pool);
		let pool = pool_arc.read().unwrap();
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

// Push new transactions to our stem transaction pool, that should broadcast it
// to the network if valid.
struct PoolPushHandler<T> {
	peers: Weak<p2p::Peers>,
	tx_pool: Weak<RwLock<pool::TransactionPool<T>>>,
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

		let mut fluff = false;
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(_) = params.get("fluff") {
				fluff = true;
			}
		}

		// Will not do a stem transaction if our dandelion peer relay is empty
		if !fluff && w(&self.peers).get_dandelion_relay().is_empty() {
			debug!(
				LOGGER,
				"Missing Dandelion relay: will push stem transaction normally"
			);
			fluff = true;
		}

		//  Push into the pool or stempool
		let pool_arc = w(&self.tx_pool);
		let res = pool_arc
			.write()
			.unwrap()
			.add_to_memory_pool(source, tx, !fluff);

		match res {
			Ok(()) => Ok(Response::with(status::Ok)),
			Err(e) => {
				debug!(LOGGER, "error - {:?}", e);
				Err(IronError::from(Error::Argument(format!("{:?}", e))))
			}
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
///
/// Hyper currently has a bug that prevents clean shutdown. In order
/// to avoid having references kept forever by handlers, we only pass
/// weak references. Note that this likely means a crash if the handlers are
/// used after a server shutdown (which should normally never happen,
/// except during tests).
pub fn start_rest_apis<T>(
	addr: String,
	chain: Weak<chain::Chain>,
	tx_pool: Weak<RwLock<pool::TransactionPool<T>>>,
	peers: Weak<p2p::Peers>,
) where
	T: pool::BlockChain + Send + Sync + 'static,
{
	let _ = thread::Builder::new()
		.name("apis".to_string())
		.spawn(move || {
			// build handlers and register them under the appropriate endpoint
			let output_handler = OutputHandler {
				chain: chain.clone(),
			};
			let block_handler = BlockHandler {
				chain: chain.clone(),
			};
			let chain_tip_handler = ChainHandler {
				chain: chain.clone(),
			};
			let chain_compact_handler = ChainCompactHandler {
				chain: chain.clone(),
			};
			let header_height_handler = HeaderByHeightHandler {
				chain: chain.clone(),
			};
			let chain_validation_handler = ChainValidationHandler {
				chain: chain.clone(),
			};
			let status_handler = StatusHandler {
				chain: chain.clone(),
				peers: peers.clone(),
			};
			let txhashset_handler = TxHashSetHandler {
				chain: chain.clone(),
			};
			let pool_info_handler = PoolInfoHandler {
				tx_pool: tx_pool.clone(),
			};
			let pool_push_handler = PoolPushHandler {
				peers: peers.clone(),
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
				"get chain/compact".to_string(),
				"get chain/validate".to_string(),
				"get chain/outputs".to_string(),
				"post chain/height-index".to_string(),
				"get status".to_string(),
				"get txhashset/roots".to_string(),
				"get txhashset/lastoutputs?n=10".to_string(),
				"get txhashset/lastrangeproofs".to_string(),
				"get txhashset/lastkernels".to_string(),
				"get pool".to_string(),
				"post pool/push".to_string(),
				"post peers/a.b.c.d:p/ban".to_string(),
				"post peers/a.b.c.d:p/unban".to_string(),
				"get peers/all".to_string(),
				"get peers/connected".to_string(),
				"get peers/a.b.c.d".to_string(),
			];
			let index_handler = IndexHandler { list: route_list };

			let router = router!(
				index: get "/" => index_handler,
				blocks: get "/blocks/*" => block_handler,
				chain_tip: get "/chain" => chain_tip_handler,
				chain_compact: get "/chain/compact" => chain_compact_handler,
				chain_validate: get "/chain/validate" => chain_validation_handler,
				chain_outputs: get "/chain/outputs/*" => output_handler,
				header_height: post "/chain/height-index" => header_height_handler,
				status: get "/status" => status_handler,
				txhashset_roots: get "/txhashset/*" => txhashset_handler,
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
