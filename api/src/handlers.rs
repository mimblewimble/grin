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

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock, Weak};
use std::thread;

use failure::ResultExt;
use futures::future::{err, ok};
use futures::{Future, Stream};
use hyper::{Body, Request, Response, StatusCode};
use rest::{Error, ErrorKind};
use serde::{Deserialize, Serialize};
use serde_json;

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
			if let Ok(_) = w(&self.chain).is_unspent(&x) {
				return Ok(Output::new(&commit));
			}
		}
		Err(ErrorKind::NotFound)?
	}

	fn outputs_by_ids(&self, req: &Request<Body>) -> Vec<Output> {
		let mut commitments: Vec<String> = vec![];
		let params = form_urlencoded::parse(req.uri().query().unwrap().as_bytes())
			.into_owned()
			.collect::<Vec<(String, String)>>();

		for (k, id) in params {
			if k == "id" {
				for id in id.split(",") {
					commitments.push(id.to_owned());
				}
			}
		}

		debug!(LOGGER, "outputs_by_ids: {:?}", commitments);

		let mut outputs: Vec<Output> = vec![];
		for x in commitments {
			if let Ok(output) = self.get_output(&x) {
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
				.outputs()
				.iter()
				.filter(|output| commitments.is_empty() || commitments.contains(&output.commit))
				.map(|output| {
					OutputPrintable::from_output(
						output,
						w(&self.chain),
						Some(&header),
						include_proof,
					)
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
	fn outputs_block_batch(&self, req: &Request<Body>) -> Vec<BlockOutputs> {
		let mut commitments: Vec<Commitment> = vec![];
		let mut start_height = 1;
		let mut end_height = 1;
		let mut include_rp = false;

		let params = form_urlencoded::parse(req.uri().query().unwrap().as_bytes())
			.into_owned()
			.fold(HashMap::new(), |mut hm, (k, v)| {
				hm.entry(k).or_insert(vec![]).push(v);
				hm
			});

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
			let res = self.outputs_at_height(i, commitments.clone(), include_rp);
			if res.outputs.len() > 0 {
				return_vec.push(res);
			}
		}

		return_vec
	}
}

impl Handler for OutputHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		match req
			.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap()
		{
			"byids" => json_response(&self.outputs_by_ids(&req)),
			"byheight" => json_response(&self.outputs_block_batch(&req)),
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
	fn outputs(&self, start_index: u64, mut max: u64) -> OutputListing {
		//set a limit here
		if max > 1000 {
			max = 1000;
		}
		let outputs = w(&self.chain)
			.unspent_outputs_by_insertion_index(start_index, max)
			.unwrap();
		OutputListing {
			last_retrieved_index: outputs.0,
			highest_index: outputs.1,
			outputs: outputs
				.2
				.iter()
				.map(|x| OutputPrintable::from_output(x, w(&self.chain), None, true))
				.collect(),
		}
	}

	// return a dummy output with merkle proof for position filled out
	// (to avoid having to create a new type to pass around)
	fn get_merkle_proof_for_output(&self, id: &str) -> Result<OutputPrintable, Error> {
		let c = util::from_hex(String::from(id)).context(ErrorKind::Argument(format!(
			"Not a valid commitment: {}",
			id
		)))?;
		let commit = Commitment::from_vec(c);
		let merkle_proof = chain::Chain::get_merkle_proof_for_pos(&w(&self.chain), commit).unwrap();
		Ok(OutputPrintable {
			output_type: OutputType::Coinbase,
			commit: Commitment::from_vec(vec![]),
			spent: false,
			proof: None,
			proof_hash: "".to_string(),
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
		match req
			.uri()
			.path()
			.trim_right()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap()
		{
			"roots" => json_response_pretty(&self.get_roots()),
			"lastoutputs" => json_response_pretty(&self.get_last_n_output(last_n)),
			"lastrangeproofs" => json_response_pretty(&self.get_last_n_rangeproof(last_n)),
			"lastkernels" => json_response_pretty(&self.get_last_n_kernel(last_n)),
			"outputs" => json_response_pretty(&self.outputs(start_index, max)),
			"merkleproof" => json_response_pretty(&self.get_merkle_proof_for_output(&id).unwrap()),
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
		if let Ok(addr) = req
			.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap()
			.parse()
		{
			match w(&self.peers).get_peer(addr) {
				Ok(peer) => json_response(&peer),
				Err(_) => response(StatusCode::BAD_REQUEST, ""),
			}
		} else {
			response(
				StatusCode::BAD_REQUEST,
				format!("url unrecognized: {}", req.uri().path()),
			)
		}
	}
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		let mut path_elems = req.uri().path().trim_right_matches("/").rsplit("/");
		match path_elems.next().unwrap() {
			"ban" => {
				if let Ok(addr) = path_elems.next().unwrap().parse() {
					w(&self.peers).ban_peer(&addr, ReasonForBan::ManualBan);
					response(StatusCode::OK, "")
				} else {
					response(StatusCode::BAD_REQUEST, "bad address to ban")
				}
			}
			"unban" => {
				if let Ok(addr) = path_elems.next().unwrap().parse() {
					w(&self.peers).unban_peer(&addr);
					response(StatusCode::OK, "")
				} else {
					response(StatusCode::BAD_REQUEST, "bad address to unban")
				}
			}
			_ => response(StatusCode::BAD_REQUEST, "unrecognized command"),
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
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
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
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		json_response(&self.get_tip())
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
		w(&self.chain).validate(true).unwrap();
		response(StatusCode::OK, "")
	}
}

/// Chain compaction handler. Trigger a compaction of the chain state to regain
/// storage space.
/// GET /v1/chain/compact
pub struct ChainCompactHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for ChainCompactHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		w(&self.chain).compact().unwrap();
		response(StatusCode::OK, "")
	}
}

/// Gets block headers given either a hash or height.
/// GET /v1/headers/<hash>
/// GET /v1/headers/<height>
///
pub struct HeaderHandler {
	pub chain: Weak<chain::Chain>,
}

impl HeaderHandler {
	fn get_header(&self, input: String) -> Result<BlockHeaderPrintable, Error> {
		if let Ok(height) = input.parse() {
			match w(&self.chain).get_header_by_height(height) {
				Ok(header) => return Ok(BlockHeaderPrintable::from_header(&header)),
				Err(_) => return Err(ErrorKind::NotFound)?,
			}
		}
		check_block_param(&input)?;
		let vec = util::from_hex(input).unwrap();
		let h = Hash::from_vec(&vec);
		let header = w(&self.chain)
			.get_block_header(&h)
			.context(ErrorKind::NotFound)?;
		Ok(BlockHeaderPrintable::from_header(&header))
	}
}

/// Gets block details given either a hash or height.
/// GET /v1/blocks/<hash>
/// GET /v1/blocks/<height>
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
			&block.as_compact_block(),
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
		let vec = util::from_hex(input).unwrap();
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
		let el = req
			.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap();

		let h = self.parse_input(el.to_string());
		if h.is_err() {
			error!(LOGGER, "block_handler: bad parameter {}", el.to_string());
			return response(StatusCode::BAD_REQUEST, "");
		}

		let h = h.unwrap();

		if let Some(param) = req.uri().query() {
			if param == "compact" {
				match self.get_compact_block(&h) {
					Ok(b) => json_response(&b),
					Err(_) => {
						error!(LOGGER, "block_handler: can not get compact block {}", h);
						response(StatusCode::INTERNAL_SERVER_ERROR, "")
					}
				}
			} else {
				debug!(
					LOGGER,
					"block_handler: unsupported query parameter {}", param
				);
				response(StatusCode::BAD_REQUEST, "")
			}
		} else {
			match self.get_block(&h) {
				Ok(b) => json_response(&b),
				Err(_) => {
					error!(LOGGER, "block_handler: can not get block {}", h);
					response(StatusCode::INTERNAL_SERVER_ERROR, "")
				}
			}
		}
	}
}

impl Handler for HeaderHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		let el = req
			.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap();

		match self.get_header(el.to_string()) {
			Ok(h) => json_response(&h),
			Err(_) => {
				error!(
					LOGGER,
					"header_handler: can not get header {}",
					el.to_string()
				);
				response(StatusCode::INTERNAL_SERVER_ERROR, "")
			}
		}
	}
}

// Get basic information about the transaction pool.
struct PoolInfoHandler<T> {
	tx_pool: Weak<RwLock<pool::TransactionPool<T>>>,
}

impl<T> Handler for PoolInfoHandler<T>
where
	T: pool::BlockChain + Send + Sync,
{
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

// Push new transaction to our local transaction pool.
struct PoolPushHandler<T> {
	tx_pool: Weak<RwLock<pool::TransactionPool<T>>>,
}

impl<T> PoolPushHandler<T>
where
	T: pool::BlockChain + Send + Sync + 'static,
{
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
						.map_err(|_| ErrorKind::RequestError("Bad request".to_owned()).into())
				})
				.and_then(move |tx_bin| {
					ser::deserialize(&mut &tx_bin[..])
						.map_err(|_| ErrorKind::RequestError("Bad request".to_owned()).into())
				})
				.and_then(move |tx: Transaction| {
					let source = pool::TxSource {
						debug_name: "push-api".to_string(),
						identifier: "?.?.?.?".to_string(),
					};
					info!(
						LOGGER,
						"Pushing transaction with {} inputs and {} outputs to pool.",
						tx.inputs().len(),
						tx.outputs().len()
					);

					//  Push to tx pool.
					let mut tx_pool = pool_arc.write().unwrap();
					tx_pool
						.add_to_pool(source, tx, !fluff)
						.map_err(|_| ErrorKind::RequestError("Bad request".to_owned()).into())
				}),
		)
	}
}

impl<T> Handler for PoolPushHandler<T>
where
	T: pool::BlockChain + Send + Sync + 'static,
{
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		Box::new(
			self.update_pool(req)
				.and_then(|_| ok(just_response(StatusCode::OK, "")))
				.or_else(|_| ok(just_response(StatusCode::BAD_REQUEST, ""))),
		)
	}
}

// Utility to serialize a struct into JSON and produce a sensible Response
// out of it.
fn json_response<T>(s: &T) -> ResponseFuture
where
	T: Serialize,
{
	match serde_json::to_string(s) {
		Ok(json) => response(StatusCode::OK, json),
		Err(_) => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
	}
}

// pretty-printed version of above
fn json_response_pretty<T>(s: &T) -> ResponseFuture
where
	T: Serialize,
{
	match serde_json::to_string_pretty(s) {
		Ok(json) => response(StatusCode::OK, json),
		Err(_) => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
	}
}

fn response<T: Into<Body> + Debug>(status: StatusCode, text: T) -> ResponseFuture {
	Box::new(ok(just_response(status, text)))
}

fn just_response<T: Into<Body> + Debug>(status: StatusCode, text: T) -> Response<Body> {
	debug!(LOGGER, "HTTP API -> status: {}, text: {:?}", status, text);
	let mut resp = Response::new(text.into());
	*resp.status_mut() = status;
	resp
}

thread_local!( static ROUTER: RefCell<Option<Router>> = RefCell::new(None) );

/// Start all server HTTP handlers. Register all of them with Router
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
			let mut apis = ApiServer::new();

			ROUTER.with(|router| {
				*router.borrow_mut() =
					Some(build_router(chain, tx_pool, peers).expect("unbale to build API router"));

				info!(LOGGER, "Starting HTTP API server at {}.", addr);
				let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
				apis.start(socket_addr, &handle).unwrap_or_else(|e| {
					error!(LOGGER, "Failed to start API HTTP server: {}.", e);
				});
			});
		});
}

pub fn handle(req: Request<Body>) -> ResponseFuture {
	ROUTER.with(|router| match *router.borrow() {
		Some(ref h) => h.handle(req),
		None => {
			error!(LOGGER, "No HTTP API router configured");
			response(StatusCode::INTERNAL_SERVER_ERROR, "No router configured")
		}
	})
}

fn parse_body<T>(req: Request<Body>) -> Box<Future<Item = T, Error = Error> + Send>
where
	for<'de> T: Deserialize<'de> + Send + 'static,
{
	Box::new(
		req.into_body()
			.concat2()
			.map_err(|_e| ErrorKind::RequestError("Failed to read request".to_owned()).into())
			.and_then(|body| match serde_json::from_reader(&body.to_vec()[..]) {
				Ok(obj) => ok(obj),
				Err(_) => err(ErrorKind::RequestError("Invalid request body".to_owned()).into()),
			}),
	)
}

pub fn build_router<T>(
	chain: Weak<chain::Chain>,
	tx_pool: Weak<RwLock<pool::TransactionPool<T>>>,
	peers: Weak<p2p::Peers>,
) -> Result<Router, RouterError>
where
	T: pool::BlockChain + Send + Sync + 'static,
{
	let route_list = vec![
		"get blocks".to_string(),
		"get chain".to_string(),
		"get chain/compact".to_string(),
		"get chain/validate".to_string(),
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
	router.add_route("/v1/", Box::new(index_handler))?;
	router.add_route("/v1/blocks/*", Box::new(block_handler))?;
	router.add_route("/v1/headers/*", Box::new(header_handler))?;
	router.add_route("/v1/chain", Box::new(chain_tip_handler))?;
	router.add_route("/v1/chain/outputs/*", Box::new(output_handler))?;
	router.add_route("/v1/chain/compact", Box::new(chain_compact_handler))?;
	router.add_route("/v1/chain/validate", Box::new(chain_validation_handler))?;
	router.add_route("/v1/txhashset/*", Box::new(txhashset_handler))?;
	router.add_route("/v1/status", Box::new(status_handler))?;
	router.add_route("/v1/pool", Box::new(pool_info_handler))?;
	router.add_route("/v1/pool/push", Box::new(pool_push_handler))?;
	router.add_route("/v1/peers/all", Box::new(peers_all_handler))?;
	router.add_route("/v1/peers/connected", Box::new(peers_connected_handler))?;
	router.add_route("/v1/peers/**", Box::new(peer_handler))?;
	Ok(router)
}
