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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock, Weak};

use failure::ResultExt;
use futures::future::ok;
use futures::Future;
use hyper::{Body, Request, StatusCode};

use chain;
use core::core::hash::{Hash, Hashed};
use core::core::{OutputFeatures, OutputIdentifier, Transaction};
use core::ser;
use p2p;
use p2p::types::ReasonForBan;
use pool;
use regex::Regex;
use rest::*;
use router::{Handler, ResponseFuture, Router, RouterError};
use types::*;
use url::form_urlencoded;
use util;
use util::secp::pedersen::Commitment;
use util::LOGGER;
use web::*;

// All handlers use `Weak` references instead of `Arc` to avoid cycles that
// can never be destroyed. These 2 functions are simple helpers to reduce the
// boilerplate of dealing with `Weak`.
fn w<T>(weak: &Weak<T>) -> Arc<T> {
	weak.upgrade().unwrap()
}

/// Retrieves an output from the chain given a commit id (a tiny bit iteratively)
fn get_output(chain: &Weak<chain::Chain>, id: &str) -> Result<(Output, OutputIdentifier), Error> {
	let c = util::from_hex(String::from(id)).context(ErrorKind::Argument(format!(
		"Not a valid commitment: {}",
		id
	)))?;
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
		if let Ok(_) = w(chain).is_unspent(&x) {
			let block_height = w(chain).get_header_for_output(&x).unwrap().height;
			return Ok((Output::new(&commit, block_height), x.clone()));
		}
	}
	Err(ErrorKind::NotFound)?
}

// RESTful index of available api endpoints
// GET /v1/
struct IndexHandler {
	list: Vec<String>,
}

impl IndexHandler {}

impl Handler for IndexHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
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
		let res = get_output(&self.chain, id)?;
		Ok(res.0)
	}

	fn outputs_by_ids(&self, req: &Request<Body>) -> Result<Vec<Output>, Error> {
		let mut commitments: Vec<String> = vec![];

		let query = match req.uri().query() {
			Some(q) => q,
			None => return Err(ErrorKind::RequestError("no query string".to_owned()))?,
		};
		let params = form_urlencoded::parse(query.as_bytes())
			.into_owned()
			.collect::<Vec<(String, String)>>();

		for (k, id) in params {
			if k == "id" {
				for id in id.split(",") {
					commitments.push(id.to_owned());
				}
			}
		}

		let mut outputs: Vec<Output> = vec![];
		for x in commitments {
			if let Ok(output) = self.get_output(&x) {
				outputs.push(output);
			}
		}
		Ok(outputs)
	}

	fn outputs_at_height(
		&self,
		block_height: u64,
		commitments: Vec<Commitment>,
		include_proof: bool,
	) -> Result<BlockOutputs, Error> {
		let header = w(&self.chain)
			.get_header_by_height(block_height)
			.map_err(|_| ErrorKind::NotFound)?;

		// TODO - possible to compact away blocks we care about
		// in the period between accepting the block and refreshing the wallet
		let block = w(&self.chain)
			.get_block(&header.hash())
			.map_err(|_| ErrorKind::NotFound)?;
		let outputs = block
			.outputs()
			.iter()
			.filter(|output| commitments.is_empty() || commitments.contains(&output.commit))
			.map(|output| {
				OutputPrintable::from_output(output, w(&self.chain), Some(&header), include_proof)
			})
			.collect();

		Ok(BlockOutputs {
			header: BlockHeaderInfo::from_header(&header),
			outputs: outputs,
		})
	}

	// returns outputs for a specified range of blocks
	fn outputs_block_batch(&self, req: &Request<Body>) -> Result<Vec<BlockOutputs>, Error> {
		let mut commitments: Vec<Commitment> = vec![];
		let mut start_height = 1;
		let mut end_height = 1;
		let mut include_rp = false;

		let query = match req.uri().query() {
			Some(q) => q,
			None => return Err(ErrorKind::RequestError("no query string".to_owned()))?,
		};

		let params = form_urlencoded::parse(query.as_bytes()).into_owned().fold(
			HashMap::new(),
			|mut hm, (k, v)| {
				hm.entry(k).or_insert(vec![]).push(v);
				hm
			},
		);

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
				start_height = height
					.parse()
					.map_err(|_| ErrorKind::RequestError("invalid start_height".to_owned()))?;
			}
		}
		if let Some(heights) = params.get("end_height") {
			for height in heights {
				end_height = height
					.parse()
					.map_err(|_| ErrorKind::RequestError("invalid end_height".to_owned()))?;
			}
		}
		if let Some(_) = params.get("include_rp") {
			include_rp = true;
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
		for i in (start_height..=end_height).rev() {
			if let Ok(res) = self.outputs_at_height(i, commitments.clone(), include_rp) {
				if res.outputs.len() > 0 {
					return_vec.push(res);
				}
			}
		}

		Ok(return_vec)
	}
}

impl Handler for OutputHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		let command = match req.uri().path().trim_right_matches("/").rsplit("/").next() {
			Some(c) => c,
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
		};
		match command {
			"byids" => result_to_response(self.outputs_by_ids(&req)),
			"byheight" => result_to_response(self.outputs_block_batch(&req)),
			_ => response(StatusCode::BAD_REQUEST, ""),
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

// UTXO traversal::
// GET /v1/txhashset/outputs?start_index=1&max=100
//
// Build a merkle proof for a given pos
// GET /v1/txhashset/merkleproof?n=1

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

	// allows traversal of utxo set
	fn outputs(&self, start_index: u64, mut max: u64) -> Result<OutputListing, Error> {
		//set a limit here
		if max > 1000 {
			max = 1000;
		}
		let outputs = w(&self.chain)
			.unspent_outputs_by_insertion_index(start_index, max)
			.context(ErrorKind::NotFound)?;
		Ok(OutputListing {
			last_retrieved_index: outputs.0,
			highest_index: outputs.1,
			outputs: outputs
				.2
				.iter()
				.map(|x| OutputPrintable::from_output(x, w(&self.chain), None, true))
				.collect(),
		})
	}

	// return a dummy output with merkle proof for position filled out
	// (to avoid having to create a new type to pass around)
	fn get_merkle_proof_for_output(&self, id: &str) -> Result<OutputPrintable, Error> {
		let c = util::from_hex(String::from(id)).context(ErrorKind::Argument(format!(
			"Not a valid commitment: {}",
			id
		)))?;
		let commit = Commitment::from_vec(c);
		let merkle_proof = chain::Chain::get_merkle_proof_for_pos(&w(&self.chain), commit)
			.map_err(|_| ErrorKind::NotFound)?;
		Ok(OutputPrintable {
			output_type: OutputType::Coinbase,
			commit: Commitment::from_vec(vec![]),
			spent: false,
			proof: None,
			proof_hash: "".to_string(),
			block_height: None,
			merkle_proof: Some(merkle_proof),
		})
	}
}

impl Handler for TxHashSetHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		let mut start_index = 1;
		let mut max = 100;
		let mut id = "".to_owned();

		// TODO: probably need to set a reasonable max limit here
		let mut last_n = 10;
		if let Some(query_string) = req.uri().query() {
			let params = form_urlencoded::parse(query_string.as_bytes())
				.into_owned()
				.fold(HashMap::new(), |mut hm, (k, v)| {
					hm.entry(k).or_insert(vec![]).push(v);
					hm
				});
			if let Some(nums) = params.get("n") {
				for num in nums {
					if let Ok(n) = str::parse(num) {
						last_n = n;
					}
				}
			}
			if let Some(start_indexes) = params.get("start_index") {
				for si in start_indexes {
					if let Ok(s) = str::parse(si) {
						start_index = s;
					}
				}
			}
			if let Some(maxes) = params.get("max") {
				for ma in maxes {
					if let Ok(m) = str::parse(ma) {
						max = m;
					}
				}
			}
			if let Some(ids) = params.get("id") {
				if !ids.is_empty() {
					id = ids.last().unwrap().to_owned();
				}
			}
		}
		let command = match req
			.uri()
			.path()
			.trim_right()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
		{
			Some(c) => c,
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
		};

		match command {
			"roots" => json_response_pretty(&self.get_roots()),
			"lastoutputs" => json_response_pretty(&self.get_last_n_output(last_n)),
			"lastrangeproofs" => json_response_pretty(&self.get_last_n_rangeproof(last_n)),
			"lastkernels" => json_response_pretty(&self.get_last_n_kernel(last_n)),
			"outputs" => result_to_response(self.outputs(start_index, max)),
			"merkleproof" => result_to_response(self.get_merkle_proof_for_output(&id)),
			_ => response(StatusCode::BAD_REQUEST, ""),
		}
	}
}

pub struct PeersAllHandler {
	pub peers: Weak<p2p::Peers>,
}

impl Handler for PeersAllHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		let peers = &w(&self.peers).all_peers();
		json_response_pretty(&peers)
	}
}

pub struct PeersConnectedHandler {
	pub peers: Weak<p2p::Peers>,
}

impl Handler for PeersConnectedHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
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
/// GET /v1/peers/10.12.12.13
/// POST /v1/peers/10.12.12.13/ban
/// POST /v1/peers/10.12.12.13/unban
pub struct PeerHandler {
	pub peers: Weak<p2p::Peers>,
}

impl Handler for PeerHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		let command = match req.uri().path().trim_right_matches("/").rsplit("/").next() {
			Some(c) => c,
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
		};
		if let Ok(addr) = command.parse() {
			match w(&self.peers).get_peer(addr) {
				Ok(peer) => json_response(&peer),
				Err(_) => response(StatusCode::NOT_FOUND, "peer not found"),
			}
		} else {
			response(
				StatusCode::BAD_REQUEST,
				format!("peer address unrecognized: {}", req.uri().path()),
			)
		}
	}
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		let mut path_elems = req.uri().path().trim_right_matches("/").rsplit("/");
		let command = match path_elems.next() {
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
			Some(c) => c,
		};
		let addr = match path_elems.next() {
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
			Some(a) => match a.parse() {
				Err(e) => {
					return response(
						StatusCode::BAD_REQUEST,
						format!("invalid peer address: {}", e),
					)
				}
				Ok(addr) => addr,
			},
		};

		match command {
			"ban" => w(&self.peers).ban_peer(&addr, ReasonForBan::ManualBan),
			"unban" => w(&self.peers).unban_peer(&addr),
			_ => return response(StatusCode::BAD_REQUEST, "invalid command"),
		};

		response(StatusCode::OK, "")
	}
}

/// Status handler. Post a summary of the server status
/// GET /v1/status
pub struct StatusHandler {
	pub chain: Weak<chain::Chain>,
	pub peers: Weak<p2p::Peers>,
}

impl StatusHandler {
	fn get_status(&self) -> Result<Status, Error> {
		let head = w(&self.chain)
			.head()
			.map_err(|e| ErrorKind::Internal(format!("can't get head: {}", e)))?;
		Ok(Status::from_tip_and_peers(
			head,
			w(&self.peers).peer_count(),
		))
	}
}

impl Handler for StatusHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		result_to_response(self.get_status())
	}
}

/// Chain handler. Get the head details.
/// GET /v1/chain
pub struct ChainHandler {
	pub chain: Weak<chain::Chain>,
}

impl ChainHandler {
	fn get_tip(&self) -> Result<Tip, Error> {
		let head = w(&self.chain)
			.head()
			.map_err(|e| ErrorKind::Internal(format!("can't get head: {}", e)))?;
		Ok(Tip::from_tip(head))
	}
}

impl Handler for ChainHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		result_to_response(self.get_tip())
	}
}

/// Chain validation handler.
/// GET /v1/chain/validate
pub struct ChainValidationHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for ChainValidationHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		// TODO - read skip_rproofs from query params
		match w(&self.chain).validate(true) {
			Ok(_) => response(StatusCode::OK, ""),
			Err(e) => response(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("validate failed: {}", e),
			),
		}
	}
}

/// Chain compaction handler. Trigger a compaction of the chain state to regain
/// storage space.
/// POST /v1/chain/compact
pub struct ChainCompactHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for ChainCompactHandler {
	fn post(&self, _req: Request<Body>) -> ResponseFuture {
		match w(&self.chain).compact() {
			Ok(_) => response(StatusCode::OK, ""),
			Err(e) => response(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("compact failed: {}", e),
			),
		}
	}
}

/// Gets block headers given either a hash or height or an output commit.
/// GET /v1/headers/<hash>
/// GET /v1/headers/<height>
/// GET /v1/headers/<output commit>
///
pub struct HeaderHandler {
	pub chain: Weak<chain::Chain>,
}

impl HeaderHandler {
	fn get_header(&self, input: String) -> Result<BlockHeaderPrintable, Error> {
		// will fail quick if the provided isn't a commitment
		if let Ok(h) = self.get_header_for_output(input.clone()) {
			return Ok(h);
		}
		if let Ok(height) = input.parse() {
			match w(&self.chain).get_header_by_height(height) {
				Ok(header) => return Ok(BlockHeaderPrintable::from_header(&header)),
				Err(_) => return Err(ErrorKind::NotFound)?,
			}
		}
		check_block_param(&input)?;
		let vec = util::from_hex(input)
			.map_err(|e| ErrorKind::Argument(format!("invalid input: {}", e)))?;
		let h = Hash::from_vec(&vec);
		let header = w(&self.chain)
			.get_block_header(&h)
			.context(ErrorKind::NotFound)?;
		Ok(BlockHeaderPrintable::from_header(&header))
	}

	fn get_header_for_output(&self, commit_id: String) -> Result<BlockHeaderPrintable, Error> {
		let oid = get_output(&self.chain, &commit_id)?.1;
		match w(&self.chain).get_header_for_output(&oid) {
			Ok(header) => return Ok(BlockHeaderPrintable::from_header(&header)),
			Err(_) => return Err(ErrorKind::NotFound)?,
		}
	}
}

impl Handler for HeaderHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		let el = match req.uri().path().trim_right_matches("/").rsplit("/").next() {
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
			Some(el) => el,
		};
		result_to_response(self.get_header(el.to_string()))
	}
}

/// Gets block details given either a hash or an unspent commit
/// GET /v1/blocks/<hash>
/// GET /v1/blocks/<height>
/// GET /v1/blocks/<commit>
///
/// Optionally return results as "compact blocks" by passing "?compact" query
/// param GET /v1/blocks/<hash>?compact
pub struct BlockHandler {
	pub chain: Weak<chain::Chain>,
}

impl BlockHandler {
	fn get_block(&self, h: &Hash) -> Result<BlockPrintable, Error> {
		let block = w(&self.chain).get_block(h).context(ErrorKind::NotFound)?;
		Ok(BlockPrintable::from_block(&block, w(&self.chain), false))
	}

	fn get_compact_block(&self, h: &Hash) -> Result<CompactBlockPrintable, Error> {
		let block = w(&self.chain).get_block(h).context(ErrorKind::NotFound)?;
		Ok(CompactBlockPrintable::from_compact_block(
			&block.into(),
			w(&self.chain),
		))
	}

	// Try to decode the string as a height or a hash.
	fn parse_input(&self, input: String) -> Result<Hash, Error> {
		if let Ok(height) = input.parse() {
			match w(&self.chain).get_header_by_height(height) {
				Ok(header) => return Ok(header.hash()),
				Err(_) => return Err(ErrorKind::NotFound)?,
			}
		}
		check_block_param(&input)?;
		let vec = util::from_hex(input)
			.map_err(|e| ErrorKind::Argument(format!("invalid input: {}", e)))?;
		Ok(Hash::from_vec(&vec))
	}
}

fn check_block_param(input: &String) -> Result<(), Error> {
	lazy_static! {
		static ref RE: Regex = Regex::new(r"[0-9a-fA-F]{64}").unwrap();
	}
	if !RE.is_match(&input) {
		return Err(ErrorKind::Argument(
			"Not a valid hash or height.".to_owned(),
		))?;
	}
	return Ok(());
}

impl Handler for BlockHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		let el = match req.uri().path().trim_right_matches("/").rsplit("/").next() {
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
			Some(el) => el,
		};

		let h = match self.parse_input(el.to_string()) {
			Err(e) => {
				return response(
					StatusCode::BAD_REQUEST,
					format!("failed to parse input: {}", e),
				)
			}
			Ok(h) => h,
		};

		if let Some(param) = req.uri().query() {
			if param == "compact" {
				result_to_response(self.get_compact_block(&h))
			} else {
				response(
					StatusCode::BAD_REQUEST,
					format!("unsupported query parameter: {}", param),
				)
			}
		} else {
			result_to_response(self.get_block(&h))
		}
	}
}

/// Get basic information about the transaction pool.
/// GET /v1/pool
struct PoolInfoHandler {
	tx_pool: Weak<RwLock<pool::TransactionPool>>,
}

impl Handler for PoolInfoHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		let pool_arc = w(&self.tx_pool);
		let pool = pool_arc.read().unwrap();

		json_response(&PoolInfo {
			pool_size: pool.total_size(),
		})
	}
}

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
struct TxWrapper {
	tx_hex: String,
}

/// Push new transaction to our local transaction pool.
/// POST /v1/pool/push
struct PoolPushHandler {
	tx_pool: Weak<RwLock<pool::TransactionPool>>,
}

impl PoolPushHandler {
	fn update_pool(&self, req: Request<Body>) -> Box<Future<Item = (), Error = Error> + Send> {
		let params = match req.uri().query() {
			Some(query_string) => form_urlencoded::parse(query_string.as_bytes())
				.into_owned()
				.fold(HashMap::new(), |mut hm, (k, v)| {
					hm.entry(k).or_insert(vec![]).push(v);
					hm
				}),
			None => HashMap::new(),
		};

		let fluff = params.get("fluff").is_some();
		let pool_arc = w(&self.tx_pool).clone();

		Box::new(
			parse_body(req)
				.and_then(move |wrapper: TxWrapper| {
					util::from_hex(wrapper.tx_hex)
						.map_err(|e| ErrorKind::RequestError(format!("Bad request: {}", e)).into())
				})
				.and_then(move |tx_bin| {
					ser::deserialize(&mut &tx_bin[..])
						.map_err(|e| ErrorKind::RequestError(format!("Bad request: {}", e)).into())
				})
				.and_then(move |tx: Transaction| {
					let source = pool::TxSource {
						debug_name: "push-api".to_string(),
						identifier: "?.?.?.?".to_string(),
					};
					info!(
						LOGGER,
						"Pushing transaction {} to pool (inputs: {}, outputs: {}, kernels: {})",
						tx.hash(),
						tx.inputs().len(),
						tx.outputs().len(),
						tx.kernels().len(),
					);

					//  Push to tx pool.
					let mut tx_pool = pool_arc.write().unwrap();
					let header = tx_pool.blockchain.chain_head().unwrap();
					tx_pool
						.add_to_pool(source, tx, !fluff, &header.hash())
						.map_err(|e| {
							error!(LOGGER, "update_pool: failed with error: {:?}", e);
							ErrorKind::Internal(format!("Failed to update pool: {:?}", e)).into()
						})
				}),
		)
	}
}

impl Handler for PoolPushHandler {
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		Box::new(
			self.update_pool(req)
				.and_then(|_| ok(just_response(StatusCode::OK, "")))
				.or_else(|e| {
					ok(just_response(
						StatusCode::INTERNAL_SERVER_ERROR,
						format!("failed: {}", e),
					))
				}),
		)
	}
}

/// Start all server HTTP handlers. Register all of them with Router
/// and runs the corresponding HTTP server.
///
/// Hyper currently has a bug that prevents clean shutdown. In order
/// to avoid having references kept forever by handlers, we only pass
/// weak references. Note that this likely means a crash if the handlers are
/// used after a server shutdown (which should normally never happen,
/// except during tests).
pub fn start_rest_apis(
	addr: String,
	chain: Weak<chain::Chain>,
	tx_pool: Weak<RwLock<pool::TransactionPool>>,
	peers: Weak<p2p::Peers>,
) -> bool {
	let mut apis = ApiServer::new();

	let router = build_router(chain, tx_pool, peers).expect("unable to build API router");

	info!(LOGGER, "Starting HTTP API server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	apis.start(socket_addr, router).is_ok()
}

pub fn build_router(
	chain: Weak<chain::Chain>,
	tx_pool: Weak<RwLock<pool::TransactionPool>>,
	peers: Weak<p2p::Peers>,
) -> Result<Router, RouterError> {
	let route_list = vec![
		"get blocks".to_string(),
		"get chain".to_string(),
		"post chain/compact".to_string(),
		"post chain/validate".to_string(),
		"get chain/outputs".to_string(),
		"get status".to_string(),
		"get txhashset/roots".to_string(),
		"get txhashset/lastoutputs?n=10".to_string(),
		"get txhashset/lastrangeproofs".to_string(),
		"get txhashset/lastkernels".to_string(),
		"get txhashset/outputs?start_index=1&max=100".to_string(),
		"get pool".to_string(),
		"post pool/push".to_string(),
		"post peers/a.b.c.d:p/ban".to_string(),
		"post peers/a.b.c.d:p/unban".to_string(),
		"get peers/all".to_string(),
		"get peers/connected".to_string(),
		"get peers/a.b.c.d".to_string(),
	];
	let index_handler = IndexHandler { list: route_list };

	let output_handler = OutputHandler {
		chain: chain.clone(),
	};

	let block_handler = BlockHandler {
		chain: chain.clone(),
	};
	let header_handler = HeaderHandler {
		chain: chain.clone(),
	};
	let chain_tip_handler = ChainHandler {
		chain: chain.clone(),
	};
	let chain_compact_handler = ChainCompactHandler {
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
		tx_pool: tx_pool.clone(),
	};
	let peers_all_handler = PeersAllHandler {
		peers: peers.clone(),
	};
	let peers_connected_handler = PeersConnectedHandler {
		peers: peers.clone(),
	};
	let peer_handler = PeerHandler {
		peers: peers.clone(),
	};

	let mut router = Router::new();
	// example how we can use midlleware
	router.add_route("/v1/", Arc::new(index_handler))?;
	router.add_route("/v1/blocks/*", Arc::new(block_handler))?;
	router.add_route("/v1/headers/*", Arc::new(header_handler))?;
	router.add_route("/v1/chain", Arc::new(chain_tip_handler))?;
	router.add_route("/v1/chain/outputs/*", Arc::new(output_handler))?;
	router.add_route("/v1/chain/compact", Arc::new(chain_compact_handler))?;
	router.add_route("/v1/chain/validate", Arc::new(chain_validation_handler))?;
	router.add_route("/v1/txhashset/*", Arc::new(txhashset_handler))?;
	router.add_route("/v1/status", Arc::new(status_handler))?;
	router.add_route("/v1/pool", Arc::new(pool_info_handler))?;
	router.add_route("/v1/pool/push", Arc::new(pool_push_handler))?;
	router.add_route("/v1/peers/all", Arc::new(peers_all_handler))?;
	router.add_route("/v1/peers/connected", Arc::new(peers_connected_handler))?;
	router.add_route("/v1/peers/**", Arc::new(peer_handler))?;
	Ok(router)
}
