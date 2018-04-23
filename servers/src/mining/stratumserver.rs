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
use std::thread;
use std::time::Duration;
use std::net::{TcpListener, TcpStream};
use std::io::{ErrorKind, Write};
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use time;
use util::LOGGER;
use std::io::BufRead;
use bufstream::BufStream;
use std::sync::{Arc, Mutex, RwLock};
use serde_json;
use std::time::SystemTime;

use common::adapters::PoolToChainAdapter;
use core::core::{Block, BlockHeader};
use common::types::StratumServerConfig;
use mining::mine_block;
use chain;
use pool;
use common::stats::{StratumStats, WorkerStats};

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
	params: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcResponse {
	id: String,
	jsonrpc: String,
	method: String,
	result: Option<String>,
	error: Option<RpcError>,
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
	nonce: u64,
	pow: Vec<u32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JobTemplate {
	height: u64,
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
	stream: BufStream<TcpStream>,
	error: bool,
	authenticated: bool,
}

impl Worker {
	/// Creates a new Stratum Worker.
	pub fn new(id: String, stream: BufStream<TcpStream>) -> Worker {
		Worker {
			id: id,
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
	current_block: Block,
	current_difficulty: u64,
	workers: Arc<Mutex<Vec<Worker>>>,
	currently_syncing: Arc<AtomicBool>,
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
			config: config,
			chain: chain_ref,
			tx_pool: tx_pool,
			current_block: Block::default(),
			current_difficulty: <u64>::max_value(),
			workers: Arc::new(Mutex::new(Vec::new())),
			currently_syncing: Arc::new(AtomicBool::new(false)),
		}
	}

	// Build and return a JobTemplate for mining the current block
	fn build_block_template(&self, bh: BlockHeader) -> JobTemplate {
		// Serialize the block header into pre and post nonce strings
		let mut pre_pow_writer = mine_block::HeaderPrePowWriter::default();
		bh.write_pre_pow(&mut pre_pow_writer).unwrap();
		let pre = pre_pow_writer.as_hex_string(false);
		let job_template = JobTemplate {
			height: bh.height,
			difficulty: self.current_difficulty,
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
					let (response, err) = match request.method.as_str() {
						"login" => {
							let (response, err) = self.handle_login(request.params);
							// XXX TODO Future? - Validate username and password
							if err == false {
								workers_l[num].authenticated = true;
							}
							(response, err)
						}
						"submit" => self.handle_submit(
							request.params,
							&mut stratum_stats.worker_stats[worker_stats_id],
						),
						"keepalive" => self.handle_keepalive(),
						"getjobtemplate" => {
							if self.currently_syncing.load(Ordering::Relaxed) {
								let e = r#"{"code": -32701, "message": "Node is syncing - Please wait"}"#;
								let err = e.to_string();
								(err, true)
							} else {
								let b = self.current_block.header.clone();
								self.handle_getjobtemplate(b)
							}
						}
						"status" => {
							self.handle_status(&stratum_stats.worker_stats[worker_stats_id])
						}
						_ => {
							// Called undefined method
							let e = r#"{"code": -32601, "message": "Method not found"}"#;
							let err = e.to_string();
							(err, true)
						}
					};

					// Package the reply as RpcResponse json
					let rpc_response: String;
					if err == true {
						let rpc_err: RpcError = serde_json::from_str(&response).unwrap();
						let resp = RpcResponse {
							id: workers_l[num].id.clone(),
							jsonrpc: String::from("2.0"),
							method: request.method,
							result: None,
							error: Some(rpc_err),
						};
						rpc_response = serde_json::to_string(&resp).unwrap();
					} else {
						let resp = RpcResponse {
							id: workers_l[num].id.clone(),
							jsonrpc: String::from("2.0"),
							method: request.method,
							result: Some(response),
							error: None,
						};
						rpc_response = serde_json::to_string(&resp).unwrap();
					}

					// Send the reply
					workers_l[num].write_message(rpc_response);
				}
				None => {} // No message for us from this worker
			}
		}
	}

	// Handle STATUS message
	fn handle_status(&self, worker_stats: &WorkerStats) -> (String, bool) {
		// Return worker status in json for use by a dashboard or healthcheck.
		let status = WorkerStatus {
			id: worker_stats.id.clone(),
			height: self.current_block.header.height,
			difficulty: worker_stats.pow_difficulty,
			accepted: worker_stats.num_accepted,
			rejected: worker_stats.num_rejected,
			stale: worker_stats.num_stale,
		};
		let response = serde_json::to_string(&status).unwrap();
		return (response, false);
	}

	// Handle GETJOBTEMPLATE message
	fn handle_getjobtemplate(&self, bh: BlockHeader) -> (String, bool) {
		// Build a JobTemplate from a BlockHeader and return JSON
		let job_template = self.build_block_template(bh);
		let job_template_json = serde_json::to_string(&job_template).unwrap();
		return (job_template_json, false);
	}

	// Handle KEEPALIVE message
	fn handle_keepalive(&self) -> (String, bool) {
		return (String::from("ok"), false);
	}

	// Handle LOGIN message
	fn handle_login(&self, params: Option<String>) -> (String, bool) {
		// Extract the params string into a LoginParams struct
		let params_str = match params {
			Some(val) => val,
			None => String::from("{}"),
		};
		let _login_params: LoginParams = match serde_json::from_str(&params_str) {
			Ok(val) => val,
			Err(_e) => {
				let r = r#"{"code": -32600, "message": "Invalid Request"}"#;
				return (String::from(r), true);
			}
		};
		return (String::from("ok"), false);
	}

	// Handle SUBMIT message
	//  params contains a solved block header
	//  we are expecting real solutions at the full difficulty.
	fn handle_submit(
		&self,
		params: Option<String>,
		worker_stats: &mut WorkerStats,
	) -> (String, bool) {
		// Extract the params string into a SubmitParams struct
		let params_str = match params {
			Some(val) => val,
			None => String::from("{}"),
		};
		let submit_params: SubmitParams = match serde_json::from_str(&params_str) {
			Ok(val) => val,
			Err(_e) => {
				let r = r#"{"code": -32600, "message": "Invalid Request"}"#;
				return (String::from(r), true);
			}
		};

		let mut b: Block;
		if submit_params.height == self.current_block.header.height {
			// Reconstruct the block header with this nonce and pow added
			b = self.current_block.clone();
			b.header.nonce = submit_params.nonce;
			b.header.pow.proof_size = submit_params.pow.len();
			b.header.pow.nonces = submit_params.pow;
			info!(
				LOGGER,
				"(Server ID: {}) Found proof of work, adding block {}",
				self.id,
				b.hash()
			);
			// Submit the block to grin server (known here as "self.miner")
			let res = self.chain.process_block(b.clone(), chain::Options::MINE);
			if let Err(e) = res {
				error!(
					LOGGER,
					"(Server ID: {}) Error validating mined block: {:?}", self.id, e
				);
				worker_stats.num_rejected += 1;
				let e = r#"{"code": -1, "message": "Solution validation failed"}"#;
				let err = e.to_string();
				return (err, true);
			}
		} else {
			warn!(
				LOGGER,
				"(Server ID: {}) Found POW for block at height: {} -  but too late",
				self.id,
				submit_params.height
			);
			worker_stats.num_stale += 1;
			let e = r#"{"code": -1, "message": "Solution submitted too late"}"#;
			let err = e.to_string();
			return (err, true);
		}
		worker_stats.num_accepted += 1;
		return (String::from("ok"), false);
	} // handle submit a solution

	// Purge dead/sick workers - remove all workers marked in error state
	fn clean_workers(&mut self, stratum_stats: &mut Arc<RwLock<StratumStats>>) {
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
				return;
			}
		}
	}

	// Broadcast a jobtemplate RpcRequest to all connected workers - no response
	// expected
	fn broadcast_job(&mut self) {
		debug!(
			LOGGER,
			"(Server ID: {}) sending block {} to stratum clients",
			self.id,
			self.current_block.header.height
		);

		// Package new block into RpcRequest
		let job_template = self.build_block_template(self.current_block.header.clone());
		let job_template_json = serde_json::to_string(&job_template).unwrap();
		let job_request = RpcRequest {
			id: String::from("Stratum"),
			jsonrpc: String::from("2.0"),
			method: String::from("job"),
			params: Some(job_template_json),
		};
		let job_request_json = serde_json::to_string(&job_request).unwrap();

		// Push the new block to all connected clients
		let mut workers_l = self.workers.lock().unwrap();
		for num in 0..workers_l.len() {
			workers_l[num].write_message(job_request_json.clone());
		}
	}

	/// "main()" - Starts the stratum-server.  Creates a thread to Listens for a connection, then
	/// enters a loop, building a new block on top of the existing chain anytime required and
	/// sending that to the connected stratum miner, proxy, or pool, and accepts full solutions to
	/// be submitted.
	pub fn run_loop(
		&mut self,
		miner_config: StratumServerConfig,
		stratum_stats: Arc<RwLock<StratumStats>>,
		cuckoo_size: u32,
		proof_size: usize,
		currently_syncing: Arc<AtomicBool>,
	) {
		info!(
			LOGGER,
			"(Server ID: {}) Starting stratum server with cuckoo_size = {}, proof_size = {}",
			self.id,
			cuckoo_size,
			proof_size
		);

		self.currently_syncing = currently_syncing;

		// "globals" for this function
		let attempt_time_per_block = miner_config.attempt_time_per_block;
		let mut deadline: i64 = 0;
		// to prevent the wallet from generating a new HD key derivation for each
		// iteration, we keep the returned derivation to provide it back when
		// nothing has changed. We only want to create a key_id for each new block,
		// and reuse it when we rebuild the current block to add new tx.
		let mut key_id = None;
		let mut head = self.chain.head().unwrap();
		let mut current_hash = head.prev_block_h;
		let mut latest_hash;
		let listen_addr = miner_config.stratum_server_addr.clone().unwrap();

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
			miner_config.stratum_server_addr.unwrap()
		);

		// Main Loop
		loop {
			// If we're fallen into sync mode, (or are just starting up,
			// tell connected clients to stop what they're doing
			let mining_stopped = self.currently_syncing.load(Ordering::Relaxed);

			// Remove workers with failed connections
			self.clean_workers(&mut stratum_stats.clone());

			// get the latest chain state
			head = self.chain.head().unwrap();
			latest_hash = head.last_block_h;

			// Build a new block if:
			//    There is a new block on the chain
			// or We are rebuilding the current one to include new transactions
			// and we're not synching
			if current_hash != latest_hash || time::get_time().sec >= deadline && !mining_stopped {
				if current_hash != latest_hash {
					// A brand new block, so we will generate a new key_id
					key_id = None;
				}
				let mut wallet_listener_url: Option<String> = None;
				if !self.config.burn_reward {
					wallet_listener_url = Some(self.config.wallet_listener_url.clone());
				}

				let (new_block, block_fees) = mine_block::get_block(
					&self.chain,
					&self.tx_pool,
					key_id.clone(),
					MAX_TX.clone(),
					wallet_listener_url,
				);
				self.current_block = new_block;
				self.current_difficulty = (self.current_block.header.total_difficulty.clone()
					- head.total_difficulty.clone())
					.into_num();
				key_id = block_fees.key_id();
				current_hash = latest_hash;
				// set a new deadline for rebuilding with fresh transactions
				deadline = time::get_time().sec + attempt_time_per_block as i64;

				{
					let mut stratum_stats = stratum_stats.write().unwrap();
					stratum_stats.block_height = self.current_block.header.height;
					stratum_stats.network_difficulty = self.current_difficulty;
				}

				// Send this job to all connected workers
				self.broadcast_job();
			}

			// Handle any messages from the workers
			self.handle_rpc_requests(&mut stratum_stats.clone());

			// sleep before restarting loop
			thread::sleep(Duration::from_millis(500));
		} // Main Loop
	} // fn run_loop()
} // StratumServer
