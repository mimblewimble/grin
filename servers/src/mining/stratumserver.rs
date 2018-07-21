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

//! Mining Stratum Server
use bufstream::BufStream;
use serde;
use serde_json;
use serde_json::Value;
use std::error::Error;
use std::io::{BufRead, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};
use std::{cmp, thread};
use time;

use chain;
use common::adapters::PoolToChainAdapter;
use common::stats::{StratumStats, WorkerStats};
use common::types::{StratumServerConfig, SyncState};
use core::core::Block;
use core::{pow, global};
use keychain;
use mining::mine_block;
use pool;
use util::LOGGER;

// Max number of transactions this miner will assemble in a block
const MAX_TX: u32 = 5000;

// ----------------------------------------
// http://www.jsonrpc.org/specification
// RPC Methods

#[derive(Serialize, Deserialize, Debug)]
struct RpcRequest {
	id: String,
	jsonrpc: String,
	method: String,
	params: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcResponse {
	id: String,
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

// ----------------------------------------
// Worker Factory Thread Function

// Run in a thread. Adds new connections to the workers list
fn accept_workers(
	id: String,
	address: String,
	workers: &mut Arc<Mutex<Vec<Worker>>>,
	stratum_stats: &mut Arc<RwLock<StratumStats>>,
) {
	let listener = TcpListener::bind(address).expect("Failed to bind to listen address");
	let mut worker_id: u32 = 0;
	for stream in listener.incoming() {
		match stream {
			Ok(stream) => {
				warn!(
					LOGGER,
					"(Server ID: {}) New connection: {}",
					id,
					stream.peer_addr().unwrap()
				);
				stream
					.set_nonblocking(true)
					.expect("set_nonblocking call failed");
				let mut worker = Worker::new(worker_id.to_string(), BufStream::new(stream));
				workers.lock().unwrap().push(worker);
				// stats for this worker (worker stat objects are added and updated but never
				// removed)
				let mut worker_stats = WorkerStats::default();
				worker_stats.is_connected = true;
				worker_stats.id = worker_id.to_string();
				worker_stats.pow_difficulty = 1; // XXX TODO
				let mut stratum_stats = stratum_stats.write().unwrap();
				stratum_stats.worker_stats.push(worker_stats);
				worker_id = worker_id + 1;
			}
			Err(e) => {
				warn!(
					LOGGER,
					"(Server ID: {}) Error accepting connection: {:?}", id, e
				);
			}
		}
	}
	// close the socket server
	drop(listener);
}

// ----------------------------------------
// Worker Object - a connected stratum client - a miner, pool, proxy, etc...

pub struct Worker {
	id: String,
	agent: String,
	login: Option<String>,
	stream: BufStream<TcpStream>,
	error: bool,
	authenticated: bool,
}

impl Worker {
	/// Creates a new Stratum Worker.
	pub fn new(id: String, stream: BufStream<TcpStream>) -> Worker {
		Worker {
			id: id,
			agent: String::from(""),
			login: None,
			stream: stream,
			error: false,
			authenticated: false,
		}
	}

	// Get Message from the worker
	fn read_message(&mut self) -> Option<String> {
		// Read and return a single message or None
		let mut line = String::new();
		match self.stream.read_line(&mut line) {
			Ok(_) => {
				return Some(line);
			}
			Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
				// Not an error, just no messages ready
				return None;
			}
			Err(e) => {
				warn!(
					LOGGER,
					"(Server ID: {}) Error in connection with stratum client: {}", self.id, e
				);
				self.error = true;
				return None;
			}
		}
	}

	// Send Message to the worker
	fn write_message(&mut self, message_in: String) {
		// Write and Flush the message
		let mut message = message_in.clone();
		if !message.ends_with("\n") {
			message += "\n";
		}
		match self.stream.write(message.as_bytes()) {
			Ok(_) => match self.stream.flush() {
				Ok(_) => {}
				Err(e) => {
					warn!(
						LOGGER,
						"(Server ID: {}) Error in connection with stratum client: {}", self.id, e
					);
					self.error = true;
				}
			},
			Err(e) => {
				warn!(
					LOGGER,
					"(Server ID: {}) Error in connection with stratum client: {}", self.id, e
				);
				self.error = true;
				return;
			}
		}
	}
} // impl Worker

// ----------------------------------------
// Grin Stratum Server

pub struct StratumServer {
	id: String,
	config: StratumServerConfig,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	current_block_versions: Vec<Block>,
	current_difficulty: u64,
	minimum_share_difficulty: u64,
	current_key_id: Option<keychain::Identifier>,
	workers: Arc<Mutex<Vec<Worker>>>,
	sync_state: Arc<SyncState>,
}

impl StratumServer {
	/// Creates a new Stratum Server.
	pub fn new(
		config: StratumServerConfig,
		chain_ref: Arc<chain::Chain>,
		tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	) -> StratumServer {
		StratumServer {
			id: String::from("StratumServer"),
			minimum_share_difficulty: config.minimum_share_difficulty,
			config: config,
			chain: chain_ref,
			tx_pool: tx_pool,
			current_block_versions: Vec::new(),
			current_difficulty: <u64>::max_value(),
			current_key_id: None,
			workers: Arc::new(Mutex::new(Vec::new())),
			sync_state: Arc::new(SyncState::new()),
		}
	}

	// Build and return a JobTemplate for mining the current block
	fn build_block_template(&self) -> JobTemplate {
		let bh = self.current_block_versions.last().unwrap().header.clone();
		// Serialize the block header into pre and post nonce strings
		let mut pre_pow_writer = mine_block::HeaderPrePowWriter::default();
		bh.write_pre_pow(&mut pre_pow_writer).unwrap();
		let pre = pre_pow_writer.as_hex_string(false);
		let job_template = JobTemplate {
			height: bh.height,
			job_id: (self.current_block_versions.len() - 1) as u64,
			difficulty: self.minimum_share_difficulty,
			pre_pow: pre,
		};
		return job_template;
	}

	// Handle an RPC request message from the worker(s)
	fn handle_rpc_requests(&mut self, stratum_stats: &mut Arc<RwLock<StratumStats>>) {
		let mut workers_l = self.workers.lock().unwrap();
		for num in 0..workers_l.len() {
			match workers_l[num].read_message() {
				Some(the_message) => {
					// Decompose the request from the JSONRpc wrapper
					let request: RpcRequest = match serde_json::from_str(&the_message) {
						Ok(request) => request,
						Err(e) => {
							// not a valid JSON RpcRequest - disconnect the worker
							warn!(
								LOGGER,
								"(Server ID: {}) Failed to parse JSONRpc: {} - {:?}",
								self.id,
								e.description(),
								the_message.as_bytes(),
							);
							workers_l[num].error = true;
							continue;
						}
					};

					let mut stratum_stats = stratum_stats.write().unwrap();
					let worker_stats_id = stratum_stats
						.worker_stats
						.iter()
						.position(|r| r.id == workers_l[num].id)
						.unwrap();
					stratum_stats.worker_stats[worker_stats_id].last_seen = SystemTime::now();

					// Call the handler function for requested method
					let response = match request.method.as_str() {
						"login" => self.handle_login(request.params, &mut workers_l[num]),
						"submit" => {
							let res = self.handle_submit(
								request.params,
								&mut workers_l[num],
								&mut stratum_stats.worker_stats[worker_stats_id],
							);
							// this key_id has been used now, reset
							if let Ok((_, true)) = res {
								self.current_key_id = None;
							}
							res.map(|(v, _)| v)
						}
						"keepalive" => self.handle_keepalive(),
						"getjobtemplate" => {
							if self.sync_state.is_syncing() {
								let e = RpcError {
									code: -32000,
									message: "Node is syncing - Please wait".to_string(),
								};
								Err(serde_json::to_value(e).unwrap())
							} else {
								self.handle_getjobtemplate()
							}
						}
						"status" => {
							self.handle_status(&stratum_stats.worker_stats[worker_stats_id])
						}
						_ => {
							// Called undefined method
							let e = RpcError {
								code: -32601,
								message: "Method not found".to_string(),
							};
							Err(serde_json::to_value(e).unwrap())
						}
					};

					// Package the reply as RpcResponse json
					let rpc_response: String;
					match response {
						Err(response) => {
							let resp = RpcResponse {
								id: request.id,
								jsonrpc: String::from("2.0"),
								method: request.method,
								result: None,
								error: Some(response),
							};
							rpc_response = serde_json::to_string(&resp).unwrap();
						}
						Ok(response) => {
							let resp = RpcResponse {
								id: request.id,
								jsonrpc: String::from("2.0"),
								method: request.method,
								result: Some(response),
								error: None,
							};
							rpc_response = serde_json::to_string(&resp).unwrap();
						}
					}

					// Send the reply
					workers_l[num].write_message(rpc_response);
				}
				None => {} // No message for us from this worker
			}
		}
	}

	// Handle STATUS message
	fn handle_status(&self, worker_stats: &WorkerStats) -> Result<Value, Value> {
		// Return worker status in json for use by a dashboard or healthcheck.
		let status = WorkerStatus {
			id: worker_stats.id.clone(),
			height: self.current_block_versions.last().unwrap().header.height,
			difficulty: worker_stats.pow_difficulty,
			accepted: worker_stats.num_accepted,
			rejected: worker_stats.num_rejected,
			stale: worker_stats.num_stale,
		};
		let response = serde_json::to_value(&status).unwrap();
		return Ok(response);
	}

	// Handle GETJOBTEMPLATE message
	fn handle_getjobtemplate(&self) -> Result<Value, Value> {
		// Build a JobTemplate from a BlockHeader and return JSON
		let job_template = self.build_block_template();
		let response = serde_json::to_value(&job_template).unwrap();
		debug!(
			LOGGER,
			"(Server ID: {}) sending block {} with id {} to single worker",
			self.id,
			job_template.height,
			job_template.job_id,
		);
		return Ok(response);
	}

	// Handle KEEPALIVE message
	fn handle_keepalive(&self) -> Result<Value, Value> {
		return Ok(serde_json::to_value("ok".to_string()).unwrap());
	}

	// Handle LOGIN message
	fn handle_login(&self, params: Option<Value>, worker: &mut Worker) -> Result<Value, Value> {
		let params: LoginParams = parse_params(params)?;
		worker.login = Some(params.login);
		// XXX TODO Future - Validate password?
		worker.agent = params.agent;
		worker.authenticated = true;
		return Ok(serde_json::to_value("ok".to_string()).unwrap());
	}

	// Handle SUBMIT message
	// params contains a solved block header
	// We accept and log valid shares of all difficulty above configured minimum
	// Accepted shares that are full solutions will also be submitted to the
	// network
	fn handle_submit(
		&self,
		params: Option<Value>,
		worker: &mut Worker,
		worker_stats: &mut WorkerStats,
	) -> Result<(Value, bool), Value> {

		// Validate parameters
		let params: SubmitParams = parse_params(params)?;
		
		let share_difficulty: u64;
		let mut share_is_block = false;
		if params.height != self.current_block_versions.last().unwrap().header.height {
			// Return error status
			error!(
				LOGGER,
				"(Server ID: {}) Share at height {} submitted too late", self.id, params.height,
			);
			worker_stats.num_stale += 1;
			let e = RpcError {
				code: -32503,
				message: "Solution submitted too late".to_string(),
			};
			return Err(serde_json::to_value(e).unwrap());
		}
		// Find the correct version of the block to match this header
		let b: Option<&Block> = self.current_block_versions.get(params.job_id as usize);
		if b.is_none() {
			// Return error status
			error!(
				LOGGER,
				"(Server ID: {}) Failed to validate solution at height {}: invalid job_id {}",
				self.id,
				params.height,
				params.job_id,
			);
			worker_stats.num_rejected += 1;
			let e = RpcError {
				code: -32502,
				message: "Failed to validate solution".to_string(),
			};
			return Err(serde_json::to_value(e).unwrap());
		}
		let mut b: Block = b.unwrap().clone();
		// Reconstruct the block header with this nonce and pow added
		b.header.nonce = params.nonce;
		b.header.pow.nonces = params.pow;
		// Get share difficulty
		share_difficulty = b.header.pow.to_difficulty().to_num();
		// If the difficulty is too low its an error
		if share_difficulty < self.minimum_share_difficulty {
			// Return error status
			error!(
				LOGGER,
				"(Server ID: {}) Share rejected due to low difficulty: {}/{}",
				self.id,
				share_difficulty,
				self.minimum_share_difficulty,
			);
			worker_stats.num_rejected += 1;
			let e = RpcError {
				code: -32501,
				message: "Share rejected due to low difficulty".to_string(),
			};
			return Err(serde_json::to_value(e).unwrap());
		}
		// If the difficulty is high enough, submit it (which also validates it)
		if share_difficulty >= self.current_difficulty {
			// This is a full solution, submit it to the network
			let res = self.chain.process_block(b.clone(), chain::Options::MINE);
			if let Err(e) = res {
				// Return error status
				error!(
					LOGGER,
					"(Server ID: {}) Failed to validate solution at height {}: {:?}",
					self.id,
					params.height,
					e
				);
				worker_stats.num_rejected += 1;
				let e = RpcError {
					code: -32502,
					message: "Failed to validate solution".to_string(),
				};
				return Err(serde_json::to_value(e).unwrap());
			}
			share_is_block = true;
			// Log message to make it obvious we found a block
			warn!(
				LOGGER,
				"(Server ID: {}) Solution Found for block {} - Yay!!!", self.id, params.height
			);
		} else {
			// Do some validation but dont submit
			if !pow::verify_size(&b.header, global::min_sizeshift()) {
				// Return error status
				error!(
					LOGGER,
					"(Server ID: {}) Failed to validate share at height {} with nonce {} using job_id {}",
					self.id,
					params.height,
					b.header.nonce,
					params.job_id,
				);
				worker_stats.num_rejected += 1;
				let e = RpcError {
					code: -32502,
					message: "Failed to validate solution".to_string(),
				};
				return Err(serde_json::to_value(e).unwrap());
			}
		}
		// Log this as a valid share
		let submitted_by = match worker.login.clone() {
			None => worker.id.to_string(),
			Some(login) => login.clone(),
		};
		info!(
			LOGGER,
			"(Server ID: {}) Got share for block: hash {}, height {}, nonce {}, difficulty {}/{}, submitted by {}",
			self.id,
			b.hash(),
			b.header.height,
			b.header.nonce,
			share_difficulty,
			self.current_difficulty,
			submitted_by,
		);
		worker_stats.num_accepted += 1;
		let submit_response;
		if share_is_block {
			submit_response = format!("blockfound - {}", b.hash().to_hex());
		} else {
			submit_response = "ok".to_string();
		}
		return Ok((serde_json::to_value(submit_response).unwrap(), share_is_block));
	} // handle submit a solution

	// Purge dead/sick workers - remove all workers marked in error state
	fn clean_workers(&mut self, stratum_stats: &mut Arc<RwLock<StratumStats>>) -> usize {
		let mut start = 0;
		let mut workers_l = self.workers.lock().unwrap();
		loop {
			for num in start..workers_l.len() {
				if workers_l[num].error == true {
					warn!(
	                                        LOGGER,
	                                        "(Server ID: {}) Dropping worker: {}",
	                                        self.id,
						workers_l[num].id;
	                                );
					// Update worker stats
					let mut stratum_stats = stratum_stats.write().unwrap();
					let worker_stats_id = stratum_stats
						.worker_stats
						.iter()
						.position(|r| r.id == workers_l[num].id)
						.unwrap();
					stratum_stats.worker_stats[worker_stats_id].is_connected = false;
					// Remove the dead worker
					workers_l.remove(num);
					break;
				}
				start = num + 1;
			}
			if start >= workers_l.len() {
				let mut stratum_stats = stratum_stats.write().unwrap();
				stratum_stats.num_workers = workers_l.len();
				return stratum_stats.num_workers;
			}
		}
	}

	// Broadcast a jobtemplate RpcRequest to all connected workers - no response
	// expected
	fn broadcast_job(&mut self) {
		// Package new block into RpcRequest
		let job_template = self.build_block_template();
		let job_template_json = serde_json::to_string(&job_template).unwrap();
		// Issue #1159 - use a serde_json Value type to avoid extra quoting
		let job_template_value: Value = serde_json::from_str(&job_template_json).unwrap();
		let job_request = RpcRequest {
			id: String::from("Stratum"),
			jsonrpc: String::from("2.0"),
			method: String::from("job"),
			params: Some(job_template_value),
		};
		let job_request_json = serde_json::to_string(&job_request).unwrap();
		debug!(
			LOGGER,
			"(Server ID: {}) sending block {} with id {} to stratum clients",
			self.id,
			job_template.height,
			job_template.job_id,
		);
		// Push the new block to all connected clients
		// NOTE: We do not give a unique nonce (should we?) so miners need
		//       to choose one for themselves
		let mut workers_l = self.workers.lock().unwrap();
		for num in 0..workers_l.len() {
			workers_l[num].write_message(job_request_json.clone());
		}
	}

	/// "main()" - Starts the stratum-server.  Creates a thread to Listens for
	/// a connection, then enters a loop, building a new block on top of the
	/// existing chain anytime required and sending that to the connected
	/// stratum miner, proxy, or pool, and accepts full solutions to
	/// be submitted.
	pub fn run_loop(
		&mut self,
		stratum_stats: Arc<RwLock<StratumStats>>,
		cuckoo_size: u32,
		proof_size: usize,
		sync_state: Arc<SyncState>,
	) {
		info!(
			LOGGER,
			"(Server ID: {}) Starting stratum server with cuckoo_size = {}, proof_size = {}",
			self.id,
			cuckoo_size,
			proof_size
		);

		self.sync_state = sync_state;

		// "globals" for this function
		let attempt_time_per_block = self.config.attempt_time_per_block;
		let mut deadline: i64 = 0;
		// to prevent the wallet from generating a new HD key derivation for each
		// iteration, we keep the returned derivation to provide it back when
		// nothing has changed. We only want to create a key_id for each new block,
		// and reuse it when we rebuild the current block to add new tx.
		let mut num_workers: usize;
		let mut head = self.chain.head().unwrap();
		let mut current_hash = head.prev_block_h;
		let mut latest_hash;
		let listen_addr = self.config.stratum_server_addr.clone().unwrap();
		self.current_block_versions.push(Block::default());

		// Start a thread to accept new worker connections
		let mut workers_th = self.workers.clone();
		let id_th = self.id.clone();
		let mut stats_th = stratum_stats.clone();
		let _listener_th = thread::spawn(move || {
			accept_workers(id_th, listen_addr, &mut workers_th, &mut stats_th);
		});

		// We have started
		{
			let mut stratum_stats = stratum_stats.write().unwrap();
			stratum_stats.is_running = true;
			stratum_stats.cuckoo_size = cuckoo_size as u16;
		}

		warn!(
			LOGGER,
			"Stratum server started on {}",
			self.config.stratum_server_addr.clone().unwrap()
		);

		// Main Loop
		loop {
			// If we're fallen into sync mode, (or are just starting up,
			// tell connected clients to stop what they're doing
			let mining_stopped = self.sync_state.is_syncing();

			// Remove workers with failed connections
			num_workers = self.clean_workers(&mut stratum_stats.clone());

			// get the latest chain state
			head = self.chain.head().unwrap();
			latest_hash = head.last_block_h;

			// Build a new block if:
			//    There is a new block on the chain
			// or We are rebuilding the current one to include new transactions
			// and we're not synching
			// and there is at least one worker connected
			if (current_hash != latest_hash || time::get_time().sec >= deadline) && !mining_stopped
				&& num_workers > 0
			{
				let mut wallet_listener_url: Option<String> = None;
				if !self.config.burn_reward {
					wallet_listener_url = Some(self.config.wallet_listener_url.clone());
				}
				// If this is a new block, clear the current_block version history
				if current_hash != latest_hash {
					self.current_block_versions.clear();
				}
				// Build the new block (version)
				let (new_block, block_fees) = mine_block::get_block(
					&self.chain,
					&self.tx_pool,
					self.current_key_id.clone(),
					MAX_TX.clone(),
					wallet_listener_url,
				);
				self.current_difficulty = (new_block.header.total_difficulty.clone()
					- head.total_difficulty.clone())
					.to_num();
				self.current_key_id = block_fees.key_id();
				current_hash = latest_hash;
				// set the minimum acceptable share difficulty for this block
				self.minimum_share_difficulty = cmp::min(
					self.config.minimum_share_difficulty,
					self.current_difficulty,
				);
				// set a new deadline for rebuilding with fresh transactions
				deadline = time::get_time().sec + attempt_time_per_block as i64;

				{
					let mut stratum_stats = stratum_stats.write().unwrap();
					stratum_stats.block_height = new_block.header.height;
					stratum_stats.network_difficulty = self.current_difficulty;
				}
				// Add this new block version to our current block map
				self.current_block_versions.push(new_block);
				// Send this job to all connected workers
				self.broadcast_job();
			}

			// Handle any messages from the workers
			self.handle_rpc_requests(&mut stratum_stats.clone());

			// sleep before restarting loop
			thread::sleep(Duration::from_millis(50));
		} // Main Loop
	} // fn run_loop()
} // StratumServer


// Utility function to parse a JSON RPC parameter object, returning a proper
// error if things go wrong.
fn parse_params<T>(params: Option<Value>) -> Result<T, Value>
where
	for<'de> T: serde::Deserialize<'de>
{
	params
		.and_then(|v| serde_json::from_value(v).ok())
		.ok_or_else(|| {
			let e = RpcError {
				code: -32600,
				message: "Invalid Request".to_string(),
			};
			serde_json::to_value(e).unwrap()
		})
}

