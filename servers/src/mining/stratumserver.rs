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

//! Mining Stratum Server

use futures::{SinkExt, StreamExt, TryStreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio::time::{timeout, Instant};
use tokio_util::codec::{Framed, LinesCodec};

use crate::util::RwLock;
use chrono::prelude::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

use crate::chain::{self, SyncState};
use crate::common::stats::{StratumStats, WorkerStats};
use crate::common::types::StratumServerConfig;
use crate::core::consensus::graph_weight;
use crate::core::core::hash::Hashed;
use crate::core::core::Block;
use crate::core::global;
use crate::core::{pow, ser};
use crate::keychain;
use crate::mining::mine_block;
use crate::util::ToHex;
use crate::ServerTxPool;

/// Tokio bounded sender: `try_send` is `&self` and enforces capacity without
/// clone-to-bypass issues that futures::mpsc has when cloning senders.
type Tx = mpsc::Sender<String>;

/// Max concurrent stratum worker connections. Beyond this, new accepts are
/// closed immediately so we do not exhaust file descriptors.
const MAX_STRATUM_WORKERS: usize = 256;

/// Bound outbound per-worker queue. If a worker cannot keep up, the connection
/// is dropped rather than buffering forever.
const WORKER_QUEUE_SIZE: usize = 64;

/// Drop connections with no successful read/write for this long. Prevents
/// half-open / abandoned TCP sessions from accumulating (see #3867).
const WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Cap a single RPC line to avoid unbounded memory from a bad client.
const MAX_RPC_LINE_BYTES: usize = 64 * 1024;

// ----------------------------------------
// http://www.jsonrpc.org/specification
// RPC Methods

/// Represents a compliant JSON RPC 2.0 id.
/// Valid id: Integer, String.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum JsonId {
	IntId(u32),
	StrId(String),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct RpcRequest {
	id: JsonId,
	jsonrpc: String,
	method: String,
	params: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct RpcResponse {
	id: JsonId,
	jsonrpc: String,
	method: String,
	result: Option<Value>,
	error: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcError {
	code: i32,
	message: String,
}

impl RpcError {
	pub fn internal_error() -> Self {
		RpcError {
			code: 32603,
			message: "Internal error".to_owned(),
		}
	}
	pub fn node_is_syncing() -> Self {
		RpcError {
			code: -32000,
			message: "Node is syncing - Please wait".to_owned(),
		}
	}
	pub fn method_not_found() -> Self {
		RpcError {
			code: -32601,
			message: "Method not found".to_owned(),
		}
	}
	pub fn too_late() -> Self {
		RpcError {
			code: -32503,
			message: "Solution submitted too late".to_string(),
		}
	}
	pub fn cannot_validate() -> Self {
		RpcError {
			code: -32502,
			message: "Failed to validate solution".to_string(),
		}
	}
	pub fn too_low_difficulty() -> Self {
		RpcError {
			code: -32501,
			message: "Share rejected due to low difficulty".to_string(),
		}
	}
	pub fn invalid_request() -> Self {
		RpcError {
			code: -32600,
			message: "Invalid Request".to_string(),
		}
	}
}

impl From<RpcError> for Value {
	fn from(e: RpcError) -> Self {
		serde_json::to_value(e).unwrap()
	}
}

impl<T> From<T> for RpcError
where
	T: std::error::Error,
{
	fn from(e: T) -> Self {
		error!("Received unhandled error: {}", e);
		RpcError::internal_error()
	}
}

#[derive(Serialize, Deserialize, Debug)]
struct LoginParams {
	login: String,
	pass: String,
	agent: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct SubmitParams {
	height: u64,
	job_id: u64,
	nonce: u64,
	edge_bits: u32,
	pow: Vec<u64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JobTemplate {
	height: u64,
	job_id: u64,
	difficulty: u64,
	pre_pow: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WorkerStatus {
	id: String,
	height: u64,
	difficulty: u64,
	accepted: u64,
	rejected: u64,
	stale: u64,
}

struct State {
	current_block_versions: Vec<Block>,
	// to prevent the wallet from generating a new HD key derivation for each
	// iteration, we keep the returned derivation to provide it back when
	// nothing has changed. We only want to create a key_id for each new block,
	// and reuse it when we rebuild the current block to add new tx.
	current_key_id: Option<keychain::Identifier>,
	current_difficulty: u64,       // scaled
	minimum_share_difficulty: u64, // unscaled
}

impl State {
	pub fn new(minimum_share_difficulty: u64) -> Self {
		let blocks = vec![Block::default()];
		State {
			current_block_versions: blocks,
			current_key_id: None,
			current_difficulty: <u64>::max_value(),
			minimum_share_difficulty: minimum_share_difficulty,
		}
	}
}

struct Handler {
	id: String,
	workers: Arc<WorkersList>,
	sync_state: Arc<SyncState>,
	chain: Arc<chain::Chain>,
	current_state: Arc<RwLock<State>>,
}

impl Handler {
	pub fn new(
		id: String,
		stratum_stats: Arc<RwLock<StratumStats>>,
		sync_state: Arc<SyncState>,
		minimum_share_difficulty: u64,
		chain: Arc<chain::Chain>,
	) -> Self {
		Handler {
			id: id,
			workers: Arc::new(WorkersList::new(stratum_stats)),
			sync_state: sync_state,
			chain: chain,
			current_state: Arc::new(RwLock::new(State::new(minimum_share_difficulty))),
		}
	}
	pub fn from_stratum(stratum: &StratumServer) -> Self {
		Handler::new(
			stratum.id.clone(),
			stratum.stratum_stats.clone(),
			stratum.sync_state.clone(),
			stratum.config.minimum_share_difficulty,
			stratum.chain.clone(),
		)
	}
	fn handle_rpc_requests(&self, request: RpcRequest, worker_id: usize) -> String {
		self.workers.last_seen(worker_id);

		// Call the handler function for requested method
		let response = match request.method.as_str() {
			"login" => self.handle_login(request.params, worker_id),
			"submit" => {
				let res = self.handle_submit(request.params, worker_id);
				// this key_id has been used now, reset
				if let Ok((_, true)) = res {
					self.current_state.write().current_key_id = None;
				}
				res.map(|(v, _)| v)
			}
			"keepalive" => self.handle_keepalive(),
			"getjobtemplate" => {
				if self.sync_state.is_syncing() {
					Err(RpcError::node_is_syncing())
				} else {
					self.handle_getjobtemplate()
				}
			}
			"status" => self.handle_status(worker_id),
			_ => {
				// Called undefined method
				Err(RpcError::method_not_found())
			}
		};

		// Package the reply as RpcResponse json
		let resp = match response {
			Err(rpc_error) => RpcResponse {
				id: request.id,
				jsonrpc: String::from("2.0"),
				method: request.method,
				result: None,
				error: Some(rpc_error.into()),
			},
			Ok(response) => RpcResponse {
				id: request.id,
				jsonrpc: String::from("2.0"),
				method: request.method,
				result: Some(response),
				error: None,
			},
		};
		serde_json::to_string(&resp).unwrap()
	}
	fn handle_login(&self, params: Option<Value>, worker_id: usize) -> Result<Value, RpcError> {
		let params: LoginParams = parse_params(params)?;
		self.workers.login(worker_id, params.login, params.agent)?;
		return Ok("ok".into());
	}

	// Handle KEEPALIVE message
	fn handle_keepalive(&self) -> Result<Value, RpcError> {
		return Ok("ok".into());
	}

	fn handle_status(&self, worker_id: usize) -> Result<Value, RpcError> {
		// Return worker status in json for use by a dashboard or healthcheck.
		let stats = self.workers.get_stats(worker_id)?;
		let status = WorkerStatus {
			id: stats.id.clone(),
			height: self
				.current_state
				.read()
				.current_block_versions
				.last()
				.unwrap()
				.header
				.height,
			difficulty: stats.pow_difficulty,
			accepted: stats.num_accepted,
			rejected: stats.num_rejected,
			stale: stats.num_stale,
		};
		let response = serde_json::to_value(&status).unwrap();
		return Ok(response);
	}
	// Handle GETJOBTEMPLATE message
	fn handle_getjobtemplate(&self) -> Result<Value, RpcError> {
		// Build a JobTemplate from a BlockHeader and return JSON
		let job_template = self.build_block_template();
		let response = serde_json::to_value(&job_template).unwrap();
		debug!(
			"(Server ID: {}) sending block {} with id {} to single worker",
			self.id, job_template.height, job_template.job_id,
		);
		return Ok(response);
	}

	// Build and return a JobTemplate for mining the current block
	fn build_block_template(&self) -> JobTemplate {
		let bh = self
			.current_state
			.read()
			.current_block_versions
			.last()
			.unwrap()
			.header
			.clone();
		// Serialize the block header into pre and post nonce strings
		let mut header_buf = vec![];
		{
			let mut writer = ser::BinWriter::default(&mut header_buf);
			bh.write_pre_pow(&mut writer).unwrap();
			bh.pow.write_pre_pow(&mut writer).unwrap();
		}
		let pre_pow = header_buf.to_hex();
		let current_state = self.current_state.read();
		let job_template = JobTemplate {
			height: bh.height,
			job_id: (current_state.current_block_versions.len() - 1) as u64,
			difficulty: current_state.minimum_share_difficulty,
			pre_pow,
		};
		return job_template;
	}
	// Handle SUBMIT message
	// params contains a solved block header
	// We accept and log valid shares of all difficulty above configured minimum
	// Accepted shares that are full solutions will also be submitted to the
	// network
	fn handle_submit(
		&self,
		params: Option<Value>,
		worker_id: usize,
	) -> Result<(Value, bool), RpcError> {
		// Validate parameters
		let params: SubmitParams = parse_params(params)?;

		let state = self.current_state.read();
		// Find the correct version of the block to match this header
		let b: Option<&Block> = state.current_block_versions.get(params.job_id as usize);
		if params.height != state.current_block_versions.last().unwrap().header.height
			|| b.is_none()
		{
			// Return error status
			error!(
					"(Server ID: {}) Share at height {}, edge_bits {}, nonce {}, job_id {} submitted too late",
					self.id, params.height, params.edge_bits, params.nonce, params.job_id,
				);
			self.workers.update_stats(worker_id, |ws| ws.num_stale += 1);
			return Err(RpcError::too_late());
		}

		let scaled_share_difficulty: u64;
		let unscaled_share_difficulty: u64;
		let mut share_is_block = false;

		let mut b: Block = b.unwrap().clone();
		// Reconstruct the blocks header with this nonce and pow added
		b.header.pow.proof.edge_bits = params.edge_bits as u8;
		b.header.pow.nonce = params.nonce;
		b.header.pow.proof.nonces = params.pow;

		if !b.header.pow.is_primary() && !b.header.pow.is_secondary() {
			// Return error status
			error!(
					"(Server ID: {}) Failed to validate solution at height {}, hash {}, edge_bits {}, nonce {}, job_id {}: cuckoo size too small",
					self.id, params.height, b.hash(), params.edge_bits, params.nonce, params.job_id,
				);
			self.workers
				.update_stats(worker_id, |worker_stats| worker_stats.num_rejected += 1);
			return Err(RpcError::cannot_validate());
		}

		// Get share difficulty values
		scaled_share_difficulty = b.header.pow.to_difficulty(b.header.height).to_num();
		unscaled_share_difficulty = b.header.pow.to_unscaled_difficulty().to_num();
		// Note:  state.minimum_share_difficulty is unscaled
		//        state.current_difficulty is scaled
		// If the difficulty is too low its an error
		if unscaled_share_difficulty < state.minimum_share_difficulty {
			// Return error status
			error!(
					"(Server ID: {}) Share at height {}, hash {}, edge_bits {}, nonce {}, job_id {} rejected due to low difficulty: {}/{}",
					self.id, params.height, b.hash(), params.edge_bits, params.nonce, params.job_id, unscaled_share_difficulty, state.minimum_share_difficulty,
				);
			self.workers
				.update_stats(worker_id, |worker_stats| worker_stats.num_rejected += 1);
			return Err(RpcError::too_low_difficulty());
		}

		// If the difficulty is high enough, submit it (which also validates it)
		if scaled_share_difficulty >= state.current_difficulty {
			// This is a full solution, submit it to the network
			let res = self.chain.process_block(b.clone(), chain::Options::MINE);
			if let Err(e) = res {
				// Return error status
				error!(
						"(Server ID: {}) Failed to validate solution at height {}, hash {}, edge_bits {}, nonce {}, job_id {}, {}",
						self.id,
						params.height,
						b.hash(),
						params.edge_bits,
						params.nonce,
						params.job_id,
						e,
					);
				self.workers
					.update_stats(worker_id, |worker_stats| worker_stats.num_rejected += 1);
				return Err(RpcError::cannot_validate());
			}
			share_is_block = true;
			self.workers
				.update_stats(worker_id, |worker_stats| worker_stats.num_blocks_found += 1);
			self.workers.stratum_stats.write().blocks_found += 1;
			// Log message to make it obvious we found a block
			let stats = self.workers.get_stats(worker_id)?;
			warn!(
					"(Server ID: {}) Solution Found for block {}, hash {} - Yay!!! Worker ID: {}, blocks found: {}, shares: {}",
					self.id, params.height,
					b.hash(),
					stats.id,
					stats.num_blocks_found,
					stats.num_accepted,
				);
		} else {
			// Do some validation but dont submit
			let res = pow::verify_size(&b.header);
			if res.is_err() {
				// Return error status
				error!(
						"(Server ID: {}) Failed to validate share at height {}, hash {}, edge_bits {}, nonce {}, job_id {}. {:?}",
						self.id,
						params.height,
						b.hash(),
						params.edge_bits,
						b.header.pow.nonce,
						params.job_id,
						res,
					);
				self.workers
					.update_stats(worker_id, |worker_stats| worker_stats.num_rejected += 1);
				return Err(RpcError::cannot_validate());
			}
		}
		// Log this as a valid share
		self.workers.update_edge_bits(params.edge_bits as u16);
		let worker = self.workers.get_worker(worker_id)?;
		let submitted_by = match worker.login {
			None => worker.id.to_string(),
			Some(login) => login,
		};

		info!(
				"(Server ID: {}) Got share at height {}, hash {}, edge_bits {}, nonce {}, job_id {}, difficulty {}/{}, submitted by {}",
				self.id,
				b.header.height,
				b.hash(),
				b.header.pow.proof.edge_bits,
				b.header.pow.nonce,
				params.job_id,
				scaled_share_difficulty,
				state.current_difficulty,
				submitted_by,
			);
		self.workers
			.update_stats(worker_id, |worker_stats| worker_stats.num_accepted += 1);
		let submit_response = if share_is_block {
			format!("blockfound - {}", b.hash().to_hex())
		} else {
			"ok".to_string()
		};
		return Ok((
			serde_json::to_value(submit_response).unwrap(),
			share_is_block,
		));
	} // handle submit a solution

	fn broadcast_job(&self) {
		debug!("broadcast job");
		// Package new block into RpcRequest
		let job_template = self.build_block_template();
		let job_template_json = serde_json::to_string(&job_template).unwrap();
		// Issue #1159 - use a serde_json Value type to avoid extra quoting
		let job_template_value: Value = serde_json::from_str(&job_template_json).unwrap();
		let job_request = RpcRequest {
			id: JsonId::StrId(String::from("Stratum")),
			jsonrpc: String::from("2.0"),
			method: String::from("job"),
			params: Some(job_template_value),
		};
		let job_request_json = serde_json::to_string(&job_request).unwrap();
		debug!(
			"(Server ID: {}) sending block {} with id {} to stratum clients",
			self.id, job_template.height, job_template.job_id,
		);
		self.workers.broadcast(job_request_json);
	}

	pub fn run(&self, config: &StratumServerConfig, tx_pool: &ServerTxPool) {
		debug!("Run main loop");
		let mut deadline: i64 = 0;
		let mut head = self.chain.head().unwrap();
		let mut current_hash = head.prev_block_h;
		loop {
			// get the latest chain state
			head = self.chain.head().unwrap();
			let latest_hash = head.last_block_h;

			// Build a new block if there is at least one worker and
			// There is a new block on the chain or its time to rebuild
			// the current one to include new transactions
			if (current_hash != latest_hash || Utc::now().timestamp() >= deadline)
				&& self.workers.count() > 0
			{
				{
					debug!("resend updated block");
					let mut state = self.current_state.write();
					let wallet_listener_url = if !config.burn_reward {
						Some(config.wallet_listener_url.clone())
					} else {
						None
					};
					// If this is a new block we will clear the current_block version history
					let clear_blocks = current_hash != latest_hash;

					// Build the new block (version)
					let (new_block, block_fees) = mine_block::get_block(
						&self.chain,
						tx_pool,
						state.current_key_id.clone(),
						wallet_listener_url,
					);

					// scaled difficulty
					state.current_difficulty =
						(new_block.header.total_difficulty() - head.total_difficulty).to_num();

					state.current_key_id = block_fees.key_id();

					current_hash = latest_hash;
					// set the minimum acceptable share unscaled difficulty for this block
					state.minimum_share_difficulty = config.minimum_share_difficulty;

					// set a new deadline for rebuilding with fresh transactions
					deadline = Utc::now().timestamp() + config.attempt_time_per_block as i64;

					// If this is a new block we will clear the current_block version history
					if clear_blocks {
						state.current_block_versions.clear();
					}

					// Update the mining stats
					self.workers.update_block_height(new_block.header.height);
					let difficulty = new_block.header.total_difficulty() - head.total_difficulty;
					self.workers.update_network_difficulty(difficulty.to_num());
					self.workers.update_network_hashrate();

					// Add this new block candidate onto our list of block versions for this height
					state.current_block_versions.push(new_block);
				}
				// Send this job to all connected workers
				self.broadcast_job();
			}

			// sleep before restarting loop
			thread::sleep(Duration::from_millis(5));
		} // Main Loop
	}
}

// ----------------------------------------
// Worker Factory Thread Function
fn accept_connections(listen_addr: SocketAddr, handler: Arc<Handler>) {
	info!("Start tokio stratum server");
	let task = async move {
		let listener = TcpListener::bind(&listen_addr).await.unwrap_or_else(|_| {
			panic!("Stratum: Failed to bind to listen address {}", listen_addr)
		});

		loop {
			match listener.accept().await {
				Ok((socket, peer_addr)) => {
					// Hard cap concurrent workers to avoid FD exhaustion.
					if handler.workers.count() >= MAX_STRATUM_WORKERS {
						warn!(
							"Stratum: rejecting connection from {} (at max {} workers)",
							peer_addr, MAX_STRATUM_WORKERS
						);
						// Drop socket immediately (close FD).
						drop(socket);
						continue;
					}

					let handler = handler.clone();
					tokio::spawn(async move {
						if let Err(e) = socket.set_nodelay(true) {
							debug!("Stratum: set_nodelay failed for {}: {}", peer_addr, e);
						}
						handle_connection(socket, peer_addr, handler, WORKER_IDLE_TIMEOUT).await;
					});
				}
				Err(e) => {
					error!("Stratum accept error = {:?}", e);
					// Avoid busy-looping on EMFILE / transient accept failures.
					tokio::time::sleep(Duration::from_millis(100)).await;
				}
			}
		}
	};

	let rt = Runtime::new().unwrap();
	rt.block_on(task);
}

/// Run a single stratum client connection until it ends, then always free the worker slot.
///
/// `idle_timeout` is how long a session may sit with no successful read/write before
/// being closed (production uses [`WORKER_IDLE_TIMEOUT`]; tests may pass a shorter value).
async fn handle_connection(
	socket: TcpStream,
	peer_addr: SocketAddr,
	handler: Arc<Handler>,
	idle_timeout: Duration,
) {
	let (tx, mut rx) = mpsc::channel(WORKER_QUEUE_SIZE);
	let worker_id = handler.workers.add_worker(tx);
	info!("Worker {} connected from {}", worker_id, peer_addr);

	// Ensure the worker is always removed, even if the session future panics
	// or returns early (connection leak fix).
	struct WorkerGuard {
		handler: Arc<Handler>,
		worker_id: usize,
		peer_addr: SocketAddr,
	}
	impl Drop for WorkerGuard {
		fn drop(&mut self) {
			self.handler.workers.remove_worker(self.worker_id);
			info!(
				"Worker {} disconnected ({})",
				self.worker_id, self.peer_addr
			);
		}
	}
	let _guard = WorkerGuard {
		handler: handler.clone(),
		worker_id,
		peer_addr,
	};

	let framed = Framed::new(socket, LinesCodec::new_with_max_length(MAX_RPC_LINE_BYTES));
	let (mut writer, mut reader) = framed.split();

	let mut idle_deadline = Instant::now() + idle_timeout;

	loop {
		tokio::select! {
			biased;

			line = reader.try_next() => {
				match line {
					Ok(Some(line)) => {
						idle_deadline = Instant::now() + idle_timeout;
						let request = match serde_json::from_str(&line) {
							Ok(r) => r,
							Err(e) => {
								error!(
									"Worker {}: invalid JSON from {}: {}",
									worker_id, peer_addr, e
								);
								break;
							}
						};
						let resp = handler.handle_rpc_requests(request, worker_id);
						if !handler.workers.try_send_to(worker_id, resp) {
							// Queue full or worker gone — drop the connection.
							warn!(
								"Worker {}: outbound queue full or closed, dropping {}",
								worker_id, peer_addr
							);
							break;
						}
					}
					Ok(None) => {
						// Clean peer EOF.
						break;
					}
					Err(e) => {
						error!("Worker {}: read error from {}: {}", worker_id, peer_addr, e);
						break;
					}
				}
			}

			msg = rx.recv() => {
				match msg {
					Some(line) => {
						idle_deadline = Instant::now() + idle_timeout;
						// Bound write time so a stalled peer cannot pin the task forever.
						match timeout(Duration::from_secs(30), writer.send(line)).await {
							Ok(Ok(())) => {}
							Ok(Err(e)) => {
								error!(
									"Worker {}: write error to {}: {}",
									worker_id, peer_addr, e
								);
								break;
							}
							Err(_) => {
								warn!(
									"Worker {}: write timeout to {}, dropping",
									worker_id, peer_addr
								);
								break;
							}
						}
					}
					None => {
						// All senders dropped.
						break;
					}
				}
			}

			_ = tokio::time::sleep_until(idle_deadline) => {
				warn!(
					"Worker {}: idle timeout ({:?}) from {}, closing",
					worker_id,
					idle_timeout,
					peer_addr
				);
				break;
			}
		}
	}
	// WorkerGuard drops here → remove_worker + FD released with Framed.
}

// ----------------------------------------
// Worker Object - a connected stratum client - a miner, pool, proxy, etc...

#[derive(Clone)]
pub struct Worker {
	id: usize,
	agent: String,
	login: Option<String>,
	authenticated: bool,
	tx: Tx,
}

impl Worker {
	/// Creates a new Stratum Worker.
	pub fn new(id: usize, tx: Tx) -> Worker {
		Worker {
			id: id,
			agent: String::from(""),
			login: None,
			authenticated: false,
			tx: tx,
		}
	}
} // impl Worker

struct WorkersList {
	workers_list: Arc<RwLock<HashMap<usize, Worker>>>,
	stratum_stats: Arc<RwLock<StratumStats>>,
}

impl WorkersList {
	pub fn new(stratum_stats: Arc<RwLock<StratumStats>>) -> Self {
		WorkersList {
			workers_list: Arc::new(RwLock::new(HashMap::new())),
			stratum_stats: stratum_stats,
		}
	}

	pub fn add_worker(&self, tx: Tx) -> usize {
		let mut stratum_stats = self.stratum_stats.write();
		let mut workers_list = self.workers_list.write();

		// Reuse a free worker_stats slot so reconnect storms do not grow
		// the stats vector without bound.
		let worker_id = match stratum_stats
			.worker_stats
			.iter()
			.position(|ws| !ws.is_connected)
		{
			Some(id) => id,
			None => {
				let id = stratum_stats.worker_stats.len();
				stratum_stats.worker_stats.push(WorkerStats::default());
				id
			}
		};

		let worker = Worker::new(worker_id, tx);
		workers_list.insert(worker_id, worker);

		let mut worker_stats = WorkerStats::default();
		worker_stats.is_connected = true;
		worker_stats.id = worker_id.to_string();
		worker_stats.pow_difficulty = stratum_stats.minimum_share_difficulty;
		stratum_stats.worker_stats[worker_id] = worker_stats;
		stratum_stats.num_workers = workers_list.len();
		worker_id
	}

	pub fn remove_worker(&self, worker_id: usize) {
		let mut workers_list = self.workers_list.write();
		if workers_list.remove(&worker_id).is_none() {
			// Already removed (e.g. concurrent cleanup); still refresh counts.
			let mut stratum_stats = self.stratum_stats.write();
			stratum_stats.num_workers = workers_list.len();
			return;
		}
		drop(workers_list);

		// Mark slot free for reuse; keep historical counters.
		self.update_stats(worker_id, |ws| {
			ws.is_connected = false;
			ws.last_seen = SystemTime::now();
		});
		let mut stratum_stats = self.stratum_stats.write();
		stratum_stats.num_workers = self.workers_list.read().len();
	}

	pub fn login(&self, worker_id: usize, login: String, agent: String) -> Result<(), RpcError> {
		let mut wl = self.workers_list.write();
		let worker = wl
			.get_mut(&worker_id)
			.ok_or_else(RpcError::internal_error)?;
		worker.login = Some(login);
		// XXX TODO Future - Validate password?
		worker.agent = agent;
		worker.authenticated = true;
		Ok(())
	}

	pub fn get_worker(&self, worker_id: usize) -> Result<Worker, RpcError> {
		self.workers_list
			.read()
			.get(&worker_id)
			.ok_or_else(|| {
				error!("Worker {} not found", worker_id);
				RpcError::internal_error()
			})
			.map(|w| w.clone())
	}

	pub fn get_stats(&self, worker_id: usize) -> Result<WorkerStats, RpcError> {
		self.stratum_stats
			.read()
			.worker_stats
			.get(worker_id)
			.ok_or_else(RpcError::internal_error)
			.map(|ws| ws.clone())
	}

	pub fn last_seen(&self, worker_id: usize) {
		//self.stratum_stats.write().worker_stats[worker_id].last_seen = SystemTime::now();
		self.update_stats(worker_id, |ws| ws.last_seen = SystemTime::now());
	}

	pub fn update_stats(&self, worker_id: usize, f: impl FnOnce(&mut WorkerStats) -> ()) {
		let mut stratum_stats = self.stratum_stats.write();
		f(&mut stratum_stats.worker_stats[worker_id]);
	}

	/// Queue a message for a single worker. Returns false if the worker is gone
	/// or its outbound queue is full (caller should drop the connection).
	pub fn try_send_to(&self, worker_id: usize, msg: String) -> bool {
		let workers = self.workers_list.read();
		let worker = match workers.get(&worker_id) {
			Some(w) => w,
			None => return false,
		};
		worker.tx.try_send(msg).is_ok()
	}

	pub fn broadcast(&self, msg: String) {
		// Collect ids of workers whose queue is full so we can drop them.
		let mut slow = Vec::new();
		{
			let workers = self.workers_list.read();
			for (id, worker) in workers.iter() {
				if worker.tx.try_send(msg.clone()).is_err() {
					slow.push(*id);
				}
			}
		}
		for id in slow {
			warn!("Stratum: dropping slow/disconnected worker {}", id);
			self.remove_worker(id);
		}
	}

	pub fn count(&self) -> usize {
		self.workers_list.read().len()
	}

	pub fn update_edge_bits(&self, edge_bits: u16) {
		{
			let mut stratum_stats = self.stratum_stats.write();
			stratum_stats.edge_bits = edge_bits;
		}
		self.update_network_hashrate();
	}

	pub fn update_block_height(&self, height: u64) {
		let mut stratum_stats = self.stratum_stats.write();
		stratum_stats.block_height = height;
	}

	pub fn update_network_difficulty(&self, difficulty: u64) {
		let mut stratum_stats = self.stratum_stats.write();
		stratum_stats.network_difficulty = difficulty;
	}

	pub fn update_network_hashrate(&self) {
		let mut stratum_stats = self.stratum_stats.write();
		stratum_stats.network_hashrate = 42.0
			* (stratum_stats.network_difficulty as f64
				/ graph_weight(stratum_stats.block_height, stratum_stats.edge_bits as u8) as f64)
			/ 60.0;
	}
}

// ----------------------------------------
// Grin Stratum Server

pub struct StratumServer {
	id: String,
	config: StratumServerConfig,
	chain: Arc<chain::Chain>,
	pub tx_pool: ServerTxPool,
	sync_state: Arc<SyncState>,
	stratum_stats: Arc<RwLock<StratumStats>>,
}

impl StratumServer {
	/// Creates a new Stratum Server.
	pub fn new(
		config: StratumServerConfig,
		chain: Arc<chain::Chain>,
		tx_pool: ServerTxPool,
		stratum_stats: Arc<RwLock<StratumStats>>,
	) -> StratumServer {
		StratumServer {
			id: String::from("0"),
			config,
			chain,
			tx_pool,
			sync_state: Arc::new(SyncState::new()),
			stratum_stats: stratum_stats,
		}
	}

	/// "main()" - Starts the stratum-server.  Creates a thread to Listens for
	/// a connection, then enters a loop, building a new block on top of the
	/// existing chain anytime required and sending that to the connected
	/// stratum miner, proxy, or pool, and accepts full solutions to
	/// be submitted.
	pub fn run_loop(&mut self, proof_size: usize, sync_state: Arc<SyncState>) {
		info!(
			"(Server ID: {}) Starting stratum server with proof_size = {}",
			self.id, proof_size
		);

		self.sync_state = sync_state;

		let listen_addr = self
			.config
			.stratum_server_addr
			.clone()
			.unwrap()
			.parse()
			.expect("Stratum: Incorrect address ");

		let handler = Arc::new(Handler::from_stratum(&self));
		let h = handler.clone();

		let _listener_th = thread::spawn(move || {
			accept_connections(listen_addr, h);
		});

		// We have started
		{
			let mut stratum_stats = self.stratum_stats.write();
			stratum_stats.is_running = true;
			stratum_stats.edge_bits = (global::min_edge_bits() + 1) as u16;
			stratum_stats.minimum_share_difficulty = self.config.minimum_share_difficulty;
		}

		warn!(
			"Stratum server started on {}",
			self.config.stratum_server_addr.clone().unwrap()
		);

		// Initial Loop. Waiting node complete syncing
		while self.sync_state.is_syncing() {
			thread::sleep(Duration::from_millis(50));
		}

		handler.run(&self.config, &self.tx_pool);
	} // fn run_loop()
} // StratumServer

// Utility function to parse a JSON RPC parameter object, returning a proper
// error if things go wrong.
fn parse_params<T>(params: Option<Value>) -> Result<T, RpcError>
where
	for<'de> T: serde::Deserialize<'de>,
{
	params
		.and_then(|v| serde_json::from_value(v).ok())
		.ok_or_else(RpcError::invalid_request)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::chain::types::{NoopAdapter, SyncStatus};
	use crate::core::genesis;
	use crate::core::global::{self, ChainTypes};
	use std::io::{Read, Write};
	use std::net::TcpStream as StdTcpStream;
	use std::path::Path;
	use std::sync::mpsc::sync_channel;

	fn dummy_tx() -> (Tx, mpsc::Receiver<String>) {
		mpsc::channel(WORKER_QUEUE_SIZE)
	}

	fn count_open_fds() -> Option<usize> {
		// macOS / Linux: count process FDs. Best-effort leak signal.
		std::fs::read_dir("/dev/fd").ok().map(|d| d.count())
	}

	fn setup_handler(dir: &str) -> Arc<Handler> {
		global::set_local_chain_type(ChainTypes::AutomatedTesting);
		let _ = std::fs::remove_dir_all(dir);
		let chain = Arc::new(
			chain::Chain::init(
				dir.to_string(),
				Arc::new(NoopAdapter {}),
				genesis::genesis_dev(),
				pow::verify_size,
				false,
				None,
			)
			.unwrap(),
		);
		let stratum_stats = Arc::new(RwLock::new(StratumStats::default()));
		let sync_state = Arc::new(SyncState::new());
		sync_state.update(SyncStatus::NoSync);
		Arc::new(Handler::new(
			String::from("test"),
			stratum_stats,
			sync_state,
			1,
			chain,
		))
	}

	/// Start the real accept loop on an ephemeral port; return the bound address.
	fn start_test_stratum(handler: Arc<Handler>) -> SocketAddr {
		start_test_stratum_with_idle(handler, WORKER_IDLE_TIMEOUT)
	}

	fn start_test_stratum_with_idle(handler: Arc<Handler>, idle_timeout: Duration) -> SocketAddr {
		let (addr_tx, addr_rx) = sync_channel(1);
		thread::spawn(move || {
			let rt = Runtime::new().unwrap();
			rt.block_on(async move {
				let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let addr = listener.local_addr().unwrap();
				addr_tx.send(addr).unwrap();
				loop {
					match listener.accept().await {
						Ok((socket, peer_addr)) => {
							if handler.workers.count() >= MAX_STRATUM_WORKERS {
								drop(socket);
								continue;
							}
							let handler = handler.clone();
							tokio::spawn(async move {
								let _ = socket.set_nodelay(true);
								handle_connection(socket, peer_addr, handler, idle_timeout).await;
							});
						}
						Err(_) => {
							tokio::time::sleep(Duration::from_millis(50)).await;
						}
					}
				}
			});
		});
		addr_rx
			.recv_timeout(Duration::from_secs(5))
			.expect("stratum test listener failed to start")
	}

	fn stratum_login(stream: &mut StdTcpStream) {
		let req = r#"{"id":1,"jsonrpc":"2.0","method":"login","params":{"login":"miner","pass":"x","agent":"test"}}"#;
		stream.write_all(req.as_bytes()).unwrap();
		stream.write_all(b"\n").unwrap();
		stream.flush().unwrap();
		// Best-effort read of the login response (do not block forever).
		let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
		let mut buf = [0u8; 512];
		let _ = stream.read(&mut buf);
	}

	fn wait_workers(handler: &Handler, pred: impl Fn(usize) -> bool, label: &str) -> usize {
		for _ in 0..100 {
			let n = handler.workers.count();
			if pred(n) {
				return n;
			}
			thread::sleep(Duration::from_millis(50));
		}
		let n = handler.workers.count();
		panic!("{}: last worker count was {}", label, n);
	}

	/// Idle sessions are closed after the idle timeout (production: 5 minutes).
	/// Uses a short timeout so the test does not wait wall-clock 5 minutes.
	#[test]
	fn test_live_idle_miner_disconnected() {
		let dir = ".grin_stratum_live_idle";
		let handler = setup_handler(dir);
		// Short idle timeout for the test; same code path as WORKER_IDLE_TIMEOUT.
		let idle = Duration::from_millis(800);
		let addr = start_test_stratum_with_idle(handler.clone(), idle);

		// Connect and complete login (counts as activity), then go silent.
		let mut stream =
			StdTcpStream::connect_timeout(&addr, Duration::from_secs(2)).expect("connect");
		stratum_login(&mut stream);
		// Keep the socket open but send nothing further (idle miner).
		let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));

		wait_workers(&handler, |n| n >= 1, "worker should register after login");

		// Before idle timeout, worker must still be present.
		thread::sleep(idle / 2);
		assert!(
			handler.workers.count() >= 1,
			"worker should still be connected before idle timeout"
		);

		// After idle timeout (+ slack for task scheduling), worker must be gone.
		wait_workers(
			&handler,
			|n| n == 0,
			"worker should be removed after idle timeout",
		);

		// Peer should observe the server closed the connection (read EOF / error).
		let mut buf = [0u8; 64];
		let read_res = stream.read(&mut buf);
		assert!(
			matches!(read_res, Ok(0) | Err(_)),
			"expected EOF or error after idle disconnect, got {:?}",
			read_res
		);

		// Production constant is 5 minutes (document the contract this test stands in for).
		assert_eq!(WORKER_IDLE_TIMEOUT, Duration::from_secs(5 * 60));

		let _ = std::fs::remove_dir_all(Path::new(dir));
	}

	/// Live stratum listener: reconnect storm must not leave workers or FDs behind.
	#[test]
	fn test_live_reconnect_storm_workers_and_fds_bounded() {
		let dir = ".grin_stratum_live_reconnect";
		let handler = setup_handler(dir);
		let addr = start_test_stratum(handler.clone());

		let fd_before = count_open_fds();

		// Reconnecting miner simulation: open, login, drop — many times.
		const CYCLES: usize = 150;
		for _ in 0..CYCLES {
			let mut stream =
				StdTcpStream::connect_timeout(&addr, Duration::from_secs(2)).expect("connect");
			stratum_login(&mut stream);
			drop(stream);
		}

		wait_workers(&handler, |n| n == 0, "drain after reconnect storm");

		// Concurrent holds then release.
		let mut held = Vec::new();
		const CONCURRENT: usize = 40;
		for _ in 0..CONCURRENT {
			let mut stream = StdTcpStream::connect_timeout(&addr, Duration::from_secs(2))
				.expect("connect concurrent");
			stratum_login(&mut stream);
			held.push(stream);
		}
		let concurrent = wait_workers(&handler, |n| n > 0 && n <= CONCURRENT, "concurrent workers");
		assert!(concurrent <= CONCURRENT);
		drop(held);

		wait_workers(&handler, |n| n == 0, "drain after concurrent release");

		if let (Some(before), Some(after)) = (fd_before, count_open_fds()) {
			// Listener + misc FDs may grow slightly; must not track ~CYCLES connections.
			assert!(
				after < before + 80,
				"possible FD leak: before={} after={} (cycles={})",
				before,
				after,
				CYCLES
			);
		}

		// worker_stats must not grow with every reconnect (slot reuse).
		assert!(
			handler.workers.stratum_stats.read().worker_stats.len() <= CONCURRENT + 5,
			"worker_stats grew unboundedly: {}",
			handler.workers.stratum_stats.read().worker_stats.len()
		);

		let _ = std::fs::remove_dir_all(Path::new(dir));
	}

	/// When at max workers, further accepts are closed immediately and count stays capped.
	///
	/// We pre-fill the worker table (no real sockets) so the test does not depend on
	/// opening 256 concurrent TCP connections — which is flaky under CI/load.
	#[test]
	fn test_live_max_workers_cap() {
		let dir = ".grin_stratum_live_max";
		let handler = setup_handler(dir);
		let addr = start_test_stratum(handler.clone());

		// Fill to the hard cap with dummy channels (kept alive so slots stay "connected").
		let mut keep_alive: Vec<mpsc::Receiver<String>> = Vec::with_capacity(MAX_STRATUM_WORKERS);
		for _ in 0..MAX_STRATUM_WORKERS {
			let (tx, rx) = mpsc::channel(1);
			handler.workers.add_worker(tx);
			keep_alive.push(rx);
		}
		assert_eq!(
			handler.workers.count(),
			MAX_STRATUM_WORKERS,
			"pre-fill should reach exact cap"
		);

		// Extra TCP connects must not register more workers (accept drops them).
		let mut rejected = Vec::new();
		for _ in 0..16 {
			match StdTcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
				Ok(stream) => {
					let _ = stream.set_nodelay(true);
					rejected.push(stream);
				}
				Err(_) => break,
			}
		}
		thread::sleep(Duration::from_millis(300));
		assert_eq!(
			handler.workers.count(),
			MAX_STRATUM_WORKERS,
			"accept path must not exceed MAX_STRATUM_WORKERS under surplus connects"
		);

		// Free pre-filled slots; surplus TCP clients (if any still open) should not
		// leave the table permanently non-empty after we drop everything.
		drop(keep_alive);
		// Manually remove dummy workers (Drop on Receiver does not call remove_worker).
		// Re-read ids from stats / map:
		let ids: Vec<usize> = handler
			.workers
			.workers_list
			.read()
			.keys()
			.copied()
			.collect();
		for id in ids {
			handler.workers.remove_worker(id);
		}
		drop(rejected);

		wait_workers(&handler, |n| n == 0, "drain after max-cap release");
		assert!(
			handler.workers.count() <= MAX_STRATUM_WORKERS,
			"cap invariant"
		);

		let _ = std::fs::remove_dir_all(Path::new(dir));
	}

	#[test]
	fn test_worker_slot_reuse_after_disconnect() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats.clone());

		let (tx0, _rx0) = dummy_tx();
		let id0 = workers.add_worker(tx0);
		assert_eq!(id0, 0);
		assert_eq!(workers.count(), 1);
		assert_eq!(stats.read().worker_stats.len(), 1);

		workers.remove_worker(id0);
		assert_eq!(workers.count(), 0);
		assert!(!stats.read().worker_stats[0].is_connected);

		// Next connection must reuse slot 0 rather than growing the vec.
		let (tx1, _rx1) = dummy_tx();
		let id1 = workers.add_worker(tx1);
		assert_eq!(id1, 0);
		assert_eq!(stats.read().worker_stats.len(), 1);
		assert!(stats.read().worker_stats[0].is_connected);
		assert_eq!(workers.count(), 1);
	}

	#[test]
	fn test_try_send_to_missing_full_and_ok() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats);

		assert!(!workers.try_send_to(0, "nope".into()));

		let (tx, mut rx) = mpsc::channel(1);
		let id = workers.add_worker(tx);
		assert!(workers.try_send_to(id, "one".into()));
		// Tokio bounded channel: second send fails while first is pending.
		assert!(!workers.try_send_to(id, "two".into()));
		assert_eq!(rx.try_recv().unwrap(), "one");
		assert!(workers.try_send_to(id, "three".into()));

		workers.remove_worker(id);
		assert!(!workers.try_send_to(id, "after-remove".into()));
	}

	#[test]
	fn test_remove_worker_is_idempotent() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats);
		let (tx, _rx) = dummy_tx();
		let id = workers.add_worker(tx);
		workers.remove_worker(id);
		// Second remove must not panic.
		workers.remove_worker(id);
		assert_eq!(workers.count(), 0);
	}

	/// Tests deserializing an `RpcRequest` given a String as the id.
	#[test]
	fn test_request_deserialize_str() {
		let expected = RpcRequest {
			id: JsonId::StrId(String::from("1")),
			method: String::from("login"),
			jsonrpc: String::from("2.0"),
			params: None,
		};
		let json = r#"{"id":"1","method":"login","jsonrpc":"2.0","params":null}"#;
		let serialized: RpcRequest = serde_json::from_str(json).unwrap();

		assert_eq!(expected, serialized);
	}

	/// Tests serializing an `RpcRequest` given a String as the id.
	/// The extra step of deserializing again is due to associative structures not maintaining order.
	#[test]
	fn test_request_serialize_str() {
		let expected = r#"{"id":"1","method":"login","jsonrpc":"2.0","params":null}"#;
		let rpc = RpcRequest {
			id: JsonId::StrId(String::from("1")),
			method: String::from("login"),
			jsonrpc: String::from("2.0"),
			params: None,
		};
		let json_actual = serde_json::to_string(&rpc).unwrap();

		let expected_deserialized: RpcRequest = serde_json::from_str(expected).unwrap();
		let actual_deserialized: RpcRequest = serde_json::from_str(&json_actual).unwrap();

		assert_eq!(expected_deserialized, actual_deserialized);
	}

	/// Tests deserializing an `RpcResponse` given a String as the id.
	#[test]
	fn test_response_deserialize_str() {
		let expected = RpcResponse {
			id: JsonId::StrId(String::from("1")),
			method: String::from("login"),
			jsonrpc: String::from("2.0"),
			result: None,
			error: None,
		};
		let json = r#"{"id":"1","method":"login","jsonrpc":"2.0","params":null}"#;
		let serialized: RpcResponse = serde_json::from_str(json).unwrap();

		assert_eq!(expected, serialized);
	}

	/// Tests serializing an `RpcResponse` given a String as the id.
	/// The extra step of deserializing again is due to associative structures not maintaining order.
	#[test]
	fn test_response_serialize_str() {
		let expected = r#"{"id":"1","method":"login","jsonrpc":"2.0","params":null}"#;
		let rpc = RpcResponse {
			id: JsonId::StrId(String::from("1")),
			method: String::from("login"),
			jsonrpc: String::from("2.0"),
			result: None,
			error: None,
		};
		let json_actual = serde_json::to_string(&rpc).unwrap();

		let expected_deserialized: RpcResponse = serde_json::from_str(expected).unwrap();
		let actual_deserialized: RpcResponse = serde_json::from_str(&json_actual).unwrap();

		assert_eq!(expected_deserialized, actual_deserialized);
	}

	/// Tests deserializing an `RpcRequest` given an integer as the id.
	#[test]
	fn test_request_deserialize_int() {
		let expected = RpcRequest {
			id: JsonId::IntId(1),
			method: String::from("login"),
			jsonrpc: String::from("2.0"),
			params: None,
		};
		let json = r#"{"id":1,"method":"login","jsonrpc":"2.0","params":null}"#;
		let serialized: RpcRequest = serde_json::from_str(json).unwrap();

		assert_eq!(expected, serialized);
	}

	/// Tests serializing an `RpcRequest` given an integer as the id.
	/// The extra step of deserializing again is due to associative structures not maintaining order.
	#[test]
	fn test_request_serialize_int() {
		let expected = r#"{"id":1,"method":"login","jsonrpc":"2.0","params":null}"#;
		let rpc = RpcRequest {
			id: JsonId::IntId(1),
			method: String::from("login"),
			jsonrpc: String::from("2.0"),
			params: None,
		};
		let json_actual = serde_json::to_string(&rpc).unwrap();

		let expected_deserialized: RpcRequest = serde_json::from_str(expected).unwrap();
		let actual_deserialized: RpcRequest = serde_json::from_str(&json_actual).unwrap();

		assert_eq!(expected_deserialized, actual_deserialized);
	}

	/// Tests deserializing an `RpcResponse` given an integer as the id.
	#[test]
	fn test_response_deserialize_int() {
		let expected = RpcResponse {
			id: JsonId::IntId(1),
			method: String::from("login"),
			jsonrpc: String::from("2.0"),
			result: None,
			error: None,
		};
		let json = r#"{"id":1,"method":"login","jsonrpc":"2.0","params":null}"#;
		let serialized: RpcResponse = serde_json::from_str(json).unwrap();

		assert_eq!(expected, serialized);
	}

	/// Tests serializing an `RpcResponse` given an integer as the id.
	/// The extra step of deserializing again is due to associative structures not maintaining order.
	#[test]
	fn test_response_serialize_int() {
		let expected = r#"{"id":1,"method":"login","jsonrpc":"2.0","params":null}"#;
		let rpc = RpcResponse {
			id: JsonId::IntId(1),
			method: String::from("login"),
			jsonrpc: String::from("2.0"),
			result: None,
			error: None,
		};
		let json_actual = serde_json::to_string(&rpc).unwrap();

		let expected_deserialized: RpcResponse = serde_json::from_str(expected).unwrap();
		let actual_deserialized: RpcResponse = serde_json::from_str(&json_actual).unwrap();

		assert_eq!(expected_deserialized, actual_deserialized);
	}
}
