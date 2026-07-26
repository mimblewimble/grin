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
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
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

type Tx = mpsc::Sender<String>;

const MAX_STRATUM_WORKERS: usize = 256;
const WORKER_QUEUE_SIZE: usize = 64;
const WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const WORKER_WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);
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
			code: -32603,
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

struct WorkerCleanup {
	worker_id: usize,
	workers: Arc<WorkersList>,
	peer_addr: Option<SocketAddr>,
}

impl Drop for WorkerCleanup {
	fn drop(&mut self) {
		self.workers.remove_worker(self.worker_id);
		match self.peer_addr {
			Some(peer_addr) => info!("Worker {} disconnected from {}", self.worker_id, peer_addr),
			None => info!("Worker {} disconnected", self.worker_id),
		}
	}
}

async fn handle_connection(
	socket: TcpStream,
	handler: Arc<Handler>,
	_permit: OwnedSemaphorePermit,
) {
	handle_connection_with_idle_timeout(socket, handler, WORKER_IDLE_TIMEOUT).await;
}

async fn handle_connection_with_idle_timeout(
	socket: TcpStream,
	handler: Arc<Handler>,
	idle_timeout: Duration,
) {
	let peer_addr = socket.peer_addr().ok();
	let (tx, mut rx) = mpsc::channel(WORKER_QUEUE_SIZE);
	let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
	let worker_id = handler.workers.add_worker(tx, shutdown_tx);
	let _cleanup = WorkerCleanup {
		worker_id,
		workers: handler.workers.clone(),
		peer_addr,
	};

	match peer_addr {
		Some(peer_addr) => info!("Worker {} connected from {}", worker_id, peer_addr),
		None => info!("Worker {} connected", worker_id),
	}

	let framed = Framed::new(socket, LinesCodec::new_with_max_length(MAX_RPC_LINE_BYTES));
	let (mut writer, mut reader) = framed.split();
	let (activity_tx, mut activity_rx) = mpsc::channel::<()>(1);

	let reader_handler = handler.clone();
	let read_activity = activity_tx.clone();
	let read = async move {
		while let Some(line) = reader
			.try_next()
			.await
			.map_err(|e| error!("Worker {} read error: {}", worker_id, e))?
		{
			let _ = read_activity.try_send(());
			let request: RpcRequest = serde_json::from_str(&line).map_err(|e| {
				error!("Worker {} invalid JSON: {}", worker_id, e);
			})?;
			let resp = reader_handler.handle_rpc_requests(request, worker_id);
			if !reader_handler.workers.send_to(worker_id, resp).await {
				warn!("Worker {} outbound queue closed", worker_id);
				return Err(());
			}
		}
		Result::<_, ()>::Ok(())
	};

	let write = async move {
		while let Some(line) = rx.recv().await {
			match timeout(WORKER_WRITE_TIMEOUT, writer.send(line)).await {
				Ok(Ok(())) => {
					let _ = activity_tx.try_send(());
				}
				Ok(Err(e)) => {
					error!("Worker {} write error: {}", worker_id, e);
					return Err(());
				}
				Err(_) => {
					warn!("Worker {} write timed out", worker_id);
					return Err(());
				}
			}
		}
		Result::<_, ()>::Ok(())
	};

	tokio::pin!(read);
	tokio::pin!(write);
	let idle_sleep = tokio::time::sleep_until(Instant::now() + idle_timeout);
	tokio::pin!(idle_sleep);

	loop {
		tokio::select! {
			_ = &mut read => break,
			_ = &mut write => break,
			_ = &mut idle_sleep => {
				warn!("Worker {} idle for {:?}; disconnecting", worker_id, idle_timeout);
				break;
			}
			_ = shutdown_rx.recv() => break,
			activity = activity_rx.recv() => {
				if activity.is_some() {
					idle_sleep.as_mut().reset(Instant::now() + idle_timeout);
				} else {
					break;
				}
			}
		}
	}
}

async fn accept_connections_loop(listener: TcpListener, handler: Arc<Handler>) {
	let mut connections = JoinSet::new();
	let worker_limit = Arc::new(Semaphore::new(MAX_STRATUM_WORKERS));
	loop {
		tokio::select! {
			accepted = listener.accept() => {
				match accepted {
					Ok((socket, peer_addr)) => {
						let permit = match worker_limit.clone().try_acquire_owned() {
							Ok(permit) => permit,
							Err(_) => {
								warn!(
									"Stratum: rejecting connection from {} (max workers: {})",
									peer_addr, MAX_STRATUM_WORKERS
								);
								drop(socket);
								continue;
							}
						};
						let handler = handler.clone();
						connections.spawn(async move {
							if let Err(e) = socket.set_nodelay(true) {
								debug!("Stratum: set_nodelay failed for {}: {}", peer_addr, e);
							}
							handle_connection(socket, handler, permit).await;
						});
					}
					Err(e) => {
						error!("accept error = {:?}", e);
						tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
					}
				}
			}
			Some(joined) = connections.join_next(), if !connections.is_empty() => {
				if let Err(e) = joined {
					error!("stratum connection task failed: {}", e);
				}
			}
		}
	}
}

fn accept_connections(listen_addr: SocketAddr, handler: Arc<Handler>) {
	info!("Start tokio stratum server");
	let task = async move {
		let listener = TcpListener::bind(&listen_addr).await.unwrap_or_else(|_| {
			panic!("Stratum: Failed to bind to listen address {}", listen_addr)
		});
		accept_connections_loop(listener, handler).await;
	};

	let rt = Runtime::new().unwrap();
	rt.block_on(task);
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
	shutdown_tx: mpsc::Sender<()>,
}

impl Worker {
	/// Creates a new Stratum Worker.
	pub fn new(id: usize, tx: Tx, shutdown_tx: mpsc::Sender<()>) -> Worker {
		Worker {
			id: id,
			agent: String::from(""),
			login: None,
			authenticated: false,
			tx: tx,
			shutdown_tx,
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

	pub fn add_worker(&self, tx: Tx, shutdown_tx: mpsc::Sender<()>) -> usize {
		let mut stratum_stats = self.stratum_stats.write();
		let mut workers_list = self.workers_list.write();
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
		let worker = Worker::new(worker_id, tx, shutdown_tx);
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
			let mut stratum_stats = self.stratum_stats.write();
			stratum_stats.num_workers = workers_list.len();
			return;
		}
		drop(workers_list);

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

	pub async fn send_to(&self, worker_id: usize, msg: String) -> bool {
		let tx = {
			let workers_list = self.workers_list.read();
			match workers_list.get(&worker_id) {
				Some(worker) => worker.tx.clone(),
				None => return false,
			}
		};
		tx.send(msg).await.is_ok()
	}

	pub fn disconnect_worker(&self, worker_id: usize) {
		let shutdown_tx = self
			.workers_list
			.read()
			.get(&worker_id)
			.map(|worker| worker.shutdown_tx.clone());
		if let Some(shutdown_tx) = shutdown_tx {
			let _ = shutdown_tx.try_send(());
		}
	}

	pub fn broadcast(&self, msg: String) {
		let mut slow_workers = Vec::new();
		{
			let workers_list = self.workers_list.read();
			for (worker_id, worker) in workers_list.iter() {
				if worker.tx.try_send(msg.clone()).is_err() {
					slow_workers.push(*worker_id);
				}
			}
		}
		for worker_id in slow_workers {
			warn!(
				"Stratum: dropping slow or disconnected worker {}",
				worker_id
			);
			self.disconnect_worker(worker_id);
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
	use crate::chain::types::{HeaderSyncMode, NoopAdapter, SyncStatus};
	use crate::core::genesis;
	use crate::core::global::{self, ChainTypes};
	use crate::core::pow::Difficulty;
	use std::fs;
	use std::net::TcpListener as StdTcpListener;
	use std::sync::OnceLock;

	// ----------------------------------------
	// Helpers

	const TEST_MINIMUM_SHARE_DIFFICULTY: u64 = 1;

	/// Read-only chain shared by the RPC routing tests below, so the suite
	/// opens a single LMDB env. Tests that write to the chain need their own.
	fn shared_test_chain() -> Arc<chain::Chain> {
		static CHAIN: OnceLock<Arc<chain::Chain>> = OnceLock::new();
		CHAIN
			.get_or_init(|| {
				global::set_local_chain_type(ChainTypes::AutomatedTesting);
				// Under the crate-local target directory (servers/target/tmp), not
				// the workspace target/, so interrupted tests do not litter the repo root.
				let dir = "target/tmp/grin_stratum_test_shared_chain";
				let _ = fs::remove_dir_all(dir);
				Arc::new(
					chain::Chain::init(
						dir.to_string(),
						Arc::new(NoopAdapter {}),
						genesis::genesis_dev(),
						pow::verify_size,
						false,
						None,
					)
					.unwrap(),
				)
			})
			.clone()
	}

	/// Build a Handler backed by the shared test chain for RPC routing tests.
	fn shared_handler() -> Arc<Handler> {
		global::set_local_chain_type(ChainTypes::AutomatedTesting);
		let chain = shared_test_chain();
		let stratum_stats = Arc::new(RwLock::new(StratumStats::default()));
		let sync_state = Arc::new(SyncState::new());
		// Default SyncState is Initial (syncing); mark as fully synced for most tests.
		sync_state.update(SyncStatus::NoSync);
		Arc::new(Handler::new(
			String::from("test"),
			stratum_stats,
			sync_state,
			TEST_MINIMUM_SHARE_DIFFICULTY,
			chain,
		))
	}

	fn rpc_request(method: &str, params: Option<Value>) -> RpcRequest {
		RpcRequest {
			id: JsonId::IntId(1),
			jsonrpc: String::from("2.0"),
			method: method.to_string(),
			params,
		}
	}

	fn parse_rpc_response(json: &str) -> RpcResponse {
		serde_json::from_str(json).unwrap()
	}

	fn dummy_tx() -> (Tx, mpsc::Receiver<String>, mpsc::Sender<()>) {
		let (tx, rx) = mpsc::channel(WORKER_QUEUE_SIZE);
		let (shutdown_tx, _shutdown_rx) = mpsc::channel(1);
		(tx, rx, shutdown_tx)
	}

	fn add_dummy_worker(workers: &WorkersList) -> usize {
		let (tx, _rx, shutdown_tx) = dummy_tx();
		workers.add_worker(tx, shutdown_tx)
	}

	#[test]
	fn test_worker_slot_reuse_after_disconnect() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats.clone());

		let (tx0, _rx0, shutdown_tx0) = dummy_tx();
		let id0 = workers.add_worker(tx0, shutdown_tx0);
		assert_eq!(id0, 0);
		assert_eq!(workers.count(), 1);
		assert_eq!(stats.read().worker_stats.len(), 1);

		workers.remove_worker(id0);
		assert_eq!(workers.count(), 0);
		assert!(!stats.read().worker_stats[0].is_connected);

		let (tx1, _rx1, shutdown_tx1) = dummy_tx();
		let id1 = workers.add_worker(tx1, shutdown_tx1);
		assert_eq!(id1, 0);
		assert_eq!(stats.read().worker_stats.len(), 1);
		assert!(stats.read().worker_stats[0].is_connected);
		assert_eq!(workers.count(), 1);
	}

	#[test]
	fn test_send_to_missing_closed_and_ok() {
		let rt = Runtime::new().unwrap();
		rt.block_on(async {
			let stats = Arc::new(RwLock::new(StratumStats::default()));
			let workers = WorkersList::new(stats);

			assert!(!workers.send_to(0, "missing".into()).await);

			let (tx, mut rx) = mpsc::channel(1);
			let (shutdown_tx, _shutdown_rx) = mpsc::channel(1);
			let id = workers.add_worker(tx, shutdown_tx);
			assert!(workers.send_to(id, "one".into()).await);
			assert_eq!(rx.try_recv().unwrap(), "one");

			drop(rx);
			assert!(!workers.send_to(id, "closed".into()).await);

			workers.remove_worker(id);
			assert!(!workers.send_to(id, "after-remove".into()).await);
		});
	}

	#[test]
	fn test_remove_worker_is_idempotent() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats);
		let (tx, _rx, shutdown_tx) = dummy_tx();
		let id = workers.add_worker(tx, shutdown_tx);
		workers.remove_worker(id);
		workers.remove_worker(id);
		assert_eq!(workers.count(), 0);
	}

	#[test]
	fn test_accept_loop_tracks_connection_tasks() {
		let rt = Runtime::new().unwrap();
		rt.block_on(async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let addr = listener.local_addr().unwrap();
			let handler = setup_handler(".grin_stratum_accept_loop_test");

			let task = tokio::spawn(accept_connections_loop(listener, handler.clone()));
			let client = tokio::net::TcpStream::connect(addr).await.unwrap();
			for _ in 0..100 {
				if handler.workers.count() == 1 {
					break;
				}
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
			assert_eq!(handler.workers.count(), 1);
			drop(client);
			for _ in 0..100 {
				if handler.workers.count() == 0 {
					break;
				}
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
			assert_eq!(handler.workers.count(), 0);
			task.abort();
		});
	}

	#[test]
	fn test_accept_loop_enforces_max_connection_limit() {
		let rt = Runtime::new().unwrap();
		rt.block_on(async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let addr = listener.local_addr().unwrap();
			let handler = setup_handler(".grin_stratum_max_workers_test");
			let task = tokio::spawn(accept_connections_loop(listener, handler.clone()));
			let mut clients = Vec::new();
			for _ in 0..(MAX_STRATUM_WORKERS + 8) {
				if let Ok(client) = tokio::net::TcpStream::connect(addr).await {
					clients.push(client);
				}
			}
			for _ in 0..100 {
				if handler.workers.count() == MAX_STRATUM_WORKERS {
					break;
				}
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
			assert_eq!(handler.workers.count(), MAX_STRATUM_WORKERS);
			drop(clients);
			for _ in 0..100 {
				if handler.workers.count() == 0 {
					break;
				}
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
			assert_eq!(handler.workers.count(), 0);
			task.abort();
		});
	}

	#[test]
	fn test_idle_connection_is_disconnected() {
		let rt = Runtime::new().unwrap();
		rt.block_on(async {
			let server = StdTcpListener::bind("127.0.0.1:0").unwrap();
			let addr = server.local_addr().unwrap();
			let client = TcpStream::connect(addr).await.unwrap();
			let (server_socket, _) = server.accept().unwrap();
			server_socket.set_nonblocking(true).unwrap();
			let server_socket = TcpStream::from_std(server_socket).unwrap();
			let handler = setup_handler(".grin_stratum_idle_test");

			let task = tokio::spawn(handle_connection_with_idle_timeout(
				server_socket,
				handler.clone(),
				Duration::from_millis(50),
			));
			for _ in 0..100 {
				if handler.workers.count() == 1 {
					break;
				}
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
			assert_eq!(handler.workers.count(), 1);
			task.await.unwrap();
			assert_eq!(handler.workers.count(), 0);
			drop(client);
		});
	}

	#[test]
	fn test_pipelined_requests_apply_backpressure() {
		let rt = Runtime::new().unwrap();
		rt.block_on(async {
			use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

			const REQUEST_COUNT: usize = 200;

			let server = StdTcpListener::bind("127.0.0.1:0").unwrap();
			let addr = server.local_addr().unwrap();
			let mut client = TcpStream::connect(addr).await.unwrap();
			let (server_socket, _) = server.accept().unwrap();
			server_socket.set_nonblocking(true).unwrap();
			let server_socket = TcpStream::from_std(server_socket).unwrap();
			let handler = setup_handler(".grin_stratum_pipeline_test");

			let task = tokio::spawn(handle_connection_with_idle_timeout(
				server_socket,
				handler.clone(),
				Duration::from_secs(5),
			));

			let requests = (0..REQUEST_COUNT)
				.map(|id| {
					format!(
						r#"{{"id":{},"jsonrpc":"2.0","method":"keepalive","params":null}}"#,
						id
					)
				})
				.collect::<Vec<_>>()
				.join("\n") + "\n";
			client.write_all(requests.as_bytes()).await.unwrap();

			let mut lines = BufReader::new(client).lines();
			for expected_id in 0..REQUEST_COUNT {
				let line = timeout(Duration::from_secs(5), lines.next_line())
					.await
					.unwrap()
					.unwrap()
					.unwrap();
				let response: Value = serde_json::from_str(&line).unwrap();
				assert_eq!(response["id"], expected_id);
			}
			assert_eq!(handler.workers.count(), 1);

			drop(lines);
			task.await.unwrap();
			assert_eq!(handler.workers.count(), 0);
		});
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

	// ----------------------------------------
	// RpcRequest / RpcResponse serde

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

	// ----------------------------------------
	// RpcError

	#[test]
	fn test_rpc_error_constructors() {
		// Regression: internal_error() must serialize with the negative
		// JSON-RPC 2.0 code, matching api/src/json_rpc.rs.
		let value: Value = RpcError::internal_error().into();
		assert_eq!(value["code"], -32603);
	}

	// ----------------------------------------
	// parse_params

	#[test]
	fn test_parse_params_ok() {
		let login: LoginParams = parse_params(Some(serde_json::json!({
			"login": "miner1",
			"pass": "x",
			"agent": "grin-miner"
		})))
		.unwrap();
		assert_eq!(login.login, "miner1");

		let submit: SubmitParams = parse_params(Some(serde_json::json!({
			"height": 42,
			"job_id": 0,
			"nonce": 12345,
			"edge_bits": 29,
			"pow": [1, 2, 3, 4]
		})))
		.unwrap();
		assert_eq!(submit.height, 42);
		assert_eq!(submit.pow, vec![1, 2, 3, 4]);
	}

	#[test]
	fn test_parse_params_none() {
		let res: Result<LoginParams, _> = parse_params(None);
		let err = res.unwrap_err();
		assert_eq!(err.code, RpcError::invalid_request().code);
	}

	#[test]
	fn test_parse_params_wrong_shape() {
		let params = serde_json::json!({"foo": "bar"});
		let res: Result<LoginParams, _> = parse_params(Some(params));
		let err = res.unwrap_err();
		assert_eq!(err.code, RpcError::invalid_request().code);
	}

	// ----------------------------------------
	// WorkersList

	#[test]
	fn test_workers_list_add_login_remove() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats.clone());

		assert_eq!(workers.count(), 0);

		let id0 = add_dummy_worker(&workers);
		let id1 = add_dummy_worker(&workers);
		assert_eq!(id0, 0);
		assert_eq!(id1, 1);
		assert_eq!(workers.count(), 2);
		assert_eq!(stats.read().num_workers, 2);

		workers
			.login(id0, "alice".to_string(), "agent-a".to_string())
			.unwrap();
		let w = workers.get_worker(id0).unwrap();
		assert_eq!(w.login.as_deref(), Some("alice"));
		assert_eq!(w.agent, "agent-a");
		assert!(w.authenticated);

		let ws = workers.get_stats(id0).unwrap();
		assert_eq!(ws.id, "0");
		assert!(ws.is_connected);

		workers.remove_worker(id0);
		assert_eq!(workers.count(), 1);
		assert_eq!(stats.read().num_workers, 1);
		assert!(!workers.get_stats(id0).unwrap().is_connected);
		assert!(workers.get_worker(id0).is_err());
	}

	#[test]
	fn test_workers_list_relogin_replaces_login_and_agent() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats);
		let id0 = add_dummy_worker(&workers);

		workers
			.login(id0, "alice".to_string(), "agent-a".to_string())
			.unwrap();
		// A second login for the same worker replaces the previous
		// login and agent rather than being rejected.
		workers
			.login(id0, "bob".to_string(), "agent-b".to_string())
			.unwrap();

		let w = workers.get_worker(id0).unwrap();
		assert_eq!(w.login.as_deref(), Some("bob"));
		assert_eq!(w.agent, "agent-b");
		assert!(w.authenticated);
	}

	#[test]
	fn test_workers_list_login_missing_worker() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats);
		let err = workers
			.login(99, "x".to_string(), "y".to_string())
			.unwrap_err();
		assert_eq!(err.code, RpcError::internal_error().code);
	}

	#[test]
	fn test_workers_list_get_stats_missing_worker() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats);
		let _ = add_dummy_worker(&workers);

		// Index past the end of the stats vec: `get_stats` reports it as a
		// clean RpcError rather than panicking.
		let err = workers.get_stats(99).unwrap_err();
		assert_eq!(err.code, RpcError::internal_error().code);
	}

	#[test]
	fn test_workers_list_broadcast_and_send_to() {
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats);

		let (tx0, mut rx0, shutdown_tx0) = dummy_tx();
		let (tx1, mut rx1, shutdown_tx1) = dummy_tx();
		let id0 = workers.add_worker(tx0, shutdown_tx0);
		let _id1 = workers.add_worker(tx1, shutdown_tx1);

		workers.broadcast("hello-all".to_string());
		assert_eq!(rx0.try_recv().unwrap(), "hello-all");
		assert_eq!(rx1.try_recv().unwrap(), "hello-all");

		workers.send_to(id0, "hello-one".to_string());
		assert_eq!(rx0.try_recv().unwrap(), "hello-one");
		// Unicast must not deliver to the other worker (channel open but empty).
		assert!(rx1.try_recv().is_err());
	}

	#[test]
	fn test_workers_list_network_stats() {
		global::set_local_chain_type(ChainTypes::AutomatedTesting);
		let stats = Arc::new(RwLock::new(StratumStats::default()));
		let workers = WorkersList::new(stats.clone());

		workers.update_block_height(100);
		workers.update_network_difficulty(1000);
		workers.update_edge_bits(29);

		let s = stats.read();
		assert_eq!(s.block_height, 100);
		assert_eq!(s.network_difficulty, 1000);
		assert_eq!(s.edge_bits, 29);
		// hashrate is recomputed from difficulty / graph_weight
		assert!(s.network_hashrate > 0.0);
	}

	// ----------------------------------------
	// Handler RPC routing

	#[test]
	fn test_handle_keepalive() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);

		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("keepalive", None), worker_id),
		);
		assert!(resp.error.is_none());
		assert_eq!(resp.result, Some(Value::String("ok".to_string())));
		assert_eq!(resp.method, "keepalive");
	}

	#[test]
	fn test_handle_method_not_found() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);

		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("does_not_exist", None), worker_id),
		);
		assert!(resp.result.is_none());
		let err = resp.error.unwrap();
		assert_eq!(err["code"], -32601);
		assert_eq!(err["message"], "Method not found");
	}

	#[test]
	fn test_handle_login_ok() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);

		let params = serde_json::json!({
			"login": "bob",
			"pass": "secret",
			"agent": "test-agent"
		});
		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("login", Some(params)), worker_id),
		);
		assert!(resp.error.is_none());
		assert_eq!(resp.result, Some(Value::String("ok".to_string())));

		let worker = handler.workers.get_worker(worker_id).unwrap();
		assert_eq!(worker.login.as_deref(), Some("bob"));
		assert_eq!(worker.agent, "test-agent");
		assert!(worker.authenticated);
	}

	#[test]
	fn test_handle_login_invalid_params() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);

		let resp =
			parse_rpc_response(&handler.handle_rpc_requests(rpc_request("login", None), worker_id));
		assert!(resp.result.is_none());
		let err = resp.error.unwrap();
		assert_eq!(err["code"], -32600);
	}

	#[test]
	fn test_handle_getjobtemplate_while_syncing() {
		let handler = shared_handler();
		// Force syncing state
		handler.sync_state.update(SyncStatus::HeaderSync {
			sync_head: handler.chain.head().unwrap(),
			sync_mode: HeaderSyncMode::Legacy,
			highest_height: 100,
			highest_diff: Difficulty::from_num(1000),
		});
		let worker_id = add_dummy_worker(&handler.workers);

		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("getjobtemplate", None), worker_id),
		);
		assert!(resp.result.is_none());
		let err = resp.error.unwrap();
		assert_eq!(err["code"], -32000);
		assert_eq!(err["message"], "Node is syncing - Please wait");
	}

	#[test]
	fn test_handle_getjobtemplate_ok() {
		let handler = shared_handler();
		// Non-default difficulty so the template path is not only asserting the const.
		let job_difficulty = 7;
		handler.current_state.write().minimum_share_difficulty = job_difficulty;
		let worker_id = add_dummy_worker(&handler.workers);

		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("getjobtemplate", None), worker_id),
		);
		assert!(resp.error.is_none());
		let result = resp.result.unwrap();
		assert_eq!(result["height"], 0);
		assert_eq!(result["job_id"], 0);
		assert_eq!(result["difficulty"], job_difficulty);
		// pre_pow is hex-encoded header bytes.
		let pre_pow = result["pre_pow"].as_str().unwrap();
		assert!(!pre_pow.is_empty());
		assert!(pre_pow.chars().all(|c| c.is_ascii_hexdigit()));
	}

	#[test]
	fn test_handle_status() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);
		handler.workers.update_stats(worker_id, |ws| {
			ws.num_accepted = 10;
			ws.num_rejected = 2;
			ws.num_stale = 1;
			ws.pow_difficulty = 5;
		});

		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("status", None), worker_id),
		);
		assert!(resp.error.is_none());
		let result = resp.result.unwrap();
		assert_eq!(result["id"], "0");
		assert_eq!(result["height"], 0);
		assert_eq!(result["difficulty"], 5);
		assert_eq!(result["accepted"], 10);
		assert_eq!(result["rejected"], 2);
		assert_eq!(result["stale"], 1);
	}

	#[test]
	fn test_handle_submit_too_late() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);

		// Wrong height vs current block version (height 0) => stale share
		let params = serde_json::json!({
			"height": 99,
			"job_id": 0,
			"nonce": 1,
			"edge_bits": 29,
			"pow": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41]
		});
		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("submit", Some(params)), worker_id),
		);
		assert!(resp.result.is_none());
		let err = resp.error.unwrap();
		assert_eq!(err["code"], -32503);

		let ws = handler.workers.get_stats(worker_id).unwrap();
		assert_eq!(ws.num_stale, 1);
	}

	#[test]
	fn test_handle_submit_invalid_job_id() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);

		// job_id out of range of current_block_versions
		let params = serde_json::json!({
			"height": 0,
			"job_id": 99,
			"nonce": 1,
			"edge_bits": 29,
			"pow": [0, 1, 2, 3]
		});
		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("submit", Some(params)), worker_id),
		);
		assert!(resp.result.is_none());
		assert_eq!(resp.error.unwrap()["code"], -32503);
		assert_eq!(handler.workers.get_stats(worker_id).unwrap().num_stale, 1);
	}

	#[test]
	fn test_handle_submit_missing_params() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);

		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("submit", None), worker_id),
		);
		assert!(resp.result.is_none());
		assert_eq!(resp.error.unwrap()["code"], -32600);
	}

	#[test]
	fn test_handle_submit_invalid_edge_bits() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);

		// edge_bits below the AutomatedTesting minimum (10) and not the
		// secondary size (29): the proof is neither primary nor secondary, so
		// it is rejected before any cuckoo verification is attempted.
		let params = serde_json::json!({
			"height": 0,
			"job_id": 0,
			"nonce": 1,
			"edge_bits": 5,
			"pow": [0, 1, 2, 3]
		});
		let resp = parse_rpc_response(
			&handler.handle_rpc_requests(rpc_request("submit", Some(params)), worker_id),
		);
		assert!(resp.result.is_none());
		assert_eq!(resp.error.unwrap()["code"], -32502);
		assert_eq!(
			handler.workers.get_stats(worker_id).unwrap().num_rejected,
			1
		);
	}

	#[test]
	fn test_last_seen_updates() {
		let handler = shared_handler();
		let worker_id = add_dummy_worker(&handler.workers);
		// Force a known baseline instead of racing a real clock read against
		// the update below.
		handler
			.workers
			.update_stats(worker_id, |ws| ws.last_seen = SystemTime::UNIX_EPOCH);

		let _ = handler.handle_rpc_requests(rpc_request("keepalive", None), worker_id);
		let after = handler.workers.get_stats(worker_id).unwrap().last_seen;
		assert!(after > SystemTime::UNIX_EPOCH);
	}
}
