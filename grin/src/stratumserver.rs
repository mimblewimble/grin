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
use std::net::{Shutdown, TcpListener, TcpStream};
use std::io::{ErrorKind, Read, Write};
use std::mem;
use time;
use util::LOGGER;

use core::core::{Block, BlockHeader};
use core::ser;
use pow::types::MinerConfig;
use chain;
use miner::Miner;

pub struct StratumServer {
	miner: Miner,
	// Id is to identify stratum server messages in the log
	debug_output_id: String,
}

impl StratumServer {
	/// Creates a new Stratum Server.
	pub fn new(miner: Miner) -> StratumServer {
		StratumServer {
			miner: miner,
			debug_output_id: String::from("Stratum"),
		}
	}

	// Get a solution (BlockHeader) from the client
	fn get_solution(&self, stream: &mut TcpStream, stream_err: &mut bool) -> Option<BlockHeader> {
		// Get a solved block header from the stream
		// get size
		let dsz: usize;
		let mut data = [0 as u8; 4]; // using 4 byte buffer
		match stream.read_exact(&mut data) {
			Ok(_) => unsafe {
				// We got the size
				let sz = mem::transmute::<[u8; 4], u32>(data);
				dsz = sz as usize;
			},
			Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
				// This is not an 'error', just isnt any data ready for us to read right now
				return None;
			}
			Err(e) => {
				// We lost our client
				warn!(
					LOGGER,
					"(Server ID: {}) Error in connection with stratum client: {}",
					self.debug_output_id,
					e
				);
				*stream_err = true;
				return None;
			}
		}

		// get serialized data.  We know a block is coming because we just got the size,
		// so we loop for some reasonable amout of time waiting for it.
		let mut retry = 0;
		let mut dvec = vec![0; dsz];
		loop {
			match stream.read_exact(&mut dvec) {
				Ok(_) => {
					let mut bh: BlockHeader = ser::deserialize(&mut &dvec[..]).unwrap();
					return Some(bh);
				}
				Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
					// Sleep a bit and try again, but give up at some point
					retry += 1;
					if retry >= 100 {
						warn!(
							LOGGER,
							"(Server ID: {}) Error in connection with stratum client: {}",
							self.debug_output_id,
							e
						);
						*stream_err = true;
						return None;
					}
					thread::sleep(Duration::from_millis(100));
				}
				Err(e) => {
					warn!(
						LOGGER,
						"(Server ID: {}) Error in connection with stratum client: {}",
						self.debug_output_id,
						e
					);
					*stream_err = true;
					return None;
				}
			}
		}
	} // End get_solution

	// Send a new block to the stratum client for mining
	fn send_block(&self, stream: &mut TcpStream, b: &Block, stream_err: &mut bool) {
		// Serialize the block for sending over the wire
		let mut bvec = Vec::new();
		ser::serialize(&mut bvec, &b).expect("serialization failed");

		// Send the size of the block
		let bveclen: u32 = bvec.len() as u32;
		unsafe {
			let bszb = mem::transmute::<u32, [u8; 4]>(bveclen);
			match stream.write(&bszb) {
				Ok(_) => {}
				Err(e) => {
					warn!(LOGGER, "Error in connection with stratum client: {}", e);
					*stream_err = true;
					return;
				}
			}
		}

		// Send block to client
		match stream.write(&bvec) {
			Ok(_) => {
				info!(
					LOGGER,
					"(Server ID: {}) Sent block to stratum client", self.debug_output_id
				);
			}
			Err(e) => {
				warn!(
					LOGGER,
					"(Server ID: {}) Error in connection with stratum client: {}",
					self.debug_output_id,
					e
				);
				*stream_err = true;
				return;
			}
		}
	} // End send_block

	/// Starts the stratum-server.  Listens for a connection, then enters a
	/// loop, building a new block on top of the existing chain anytime required and sending that to
	/// the connected stratum miner, proxy, or pool, and accepts full solutions to be submitted.
	pub fn run_loop(&self, miner_config: MinerConfig, cuckoo_size: u32, proof_size: usize) {
		info!(
			LOGGER,
			"(Server ID: {}) Starting stratum server with cuckoo_size = {}, proof_size = {}",
			self.debug_output_id,
			cuckoo_size,
			proof_size
		);

		// "globals" for this function
		let mut b: Block = Block::default();
		let mut stream_err: bool;
		let attempt_time_per_block = miner_config.attempt_time_per_block;
		let mut deadline: i64 = 0;
		// to prevent the wallet from generating a new HD key derivation for each
		// iteration, we keep the returned derivation to provide it back when
		// nothing has changed. We only want to create on key_id for each new block,
		// but not when we rebuild the current block to add new tx.
		let mut key_id = None;

		let listen_addr = miner_config.stratum_server_addr.clone().unwrap();
		let listener = TcpListener::bind(listen_addr).unwrap();
		warn!(
			LOGGER,
			"Stratum server started on {}",
			miner_config.stratum_server_addr.unwrap()
		);

		// Outer Loop - Listen for miner connection
		for stream in listener.incoming() {
			stream_err = false;
			match stream {
				Err(e) => {
					// connection failed
					error!(
						LOGGER,
						"(Server ID: {}) Error accepting stratum connection: {:?}",
						self.debug_output_id,
						e
					);
				}
				Ok(mut stream) => {
					info!(
						LOGGER,
						"(Server ID: {}) New connection: {}",
						self.debug_output_id,
						stream.peer_addr().unwrap()
					);
					stream
						.set_nonblocking(true)
						.expect("set_nonblocking call failed");
					let mut current_hash = self.miner.chain.head().unwrap().prev_block_h;
					let mut latest_hash;

					// Inner Loop
					loop {
						trace!(
							LOGGER,
							"(Server ID: {}) key_id: {:?}",
							self.debug_output_id,
							key_id
						);

						// Abort connection on error
						if stream_err == true {
							warn!(
								LOGGER,
								"(Server ID: {}) Resetting stratum server connection",
								self.debug_output_id
							);
							match stream.shutdown(Shutdown::Both) {
								_ => {}
							}
							break;
						}

						// get the latest chain state and build a block on top of it
						latest_hash = self.miner.chain.head().unwrap().last_block_h;

						// Build a new block to mine
						if current_hash != latest_hash || time::get_time().sec >= deadline {
							// There is a new block on the chain
							// OR we are rebuilding the current one to include new transactions
							if current_hash != latest_hash {
								// A brand new block, so we will generate a new key_id
								key_id = None;
							}

							let (new_block, block_fees) = self.miner.get_block(key_id.clone());
							b = new_block;
							key_id = block_fees.key_id();
							current_hash = latest_hash;
							// set a new deadline for rebuilding with fresh transactions
							deadline = time::get_time().sec + attempt_time_per_block as i64;

							debug!(
								LOGGER,
								"(Server ID: {}) sending block {} to stratum client",
								self.debug_output_id,
								b.header.height
							);

							// Push the new block "b" to the connected client
							self.send_block(&mut stream, &b, &mut stream_err);
						}

						// Get a solved block header - if any are waiting
						// Here, we are expecting real solutions at the full difficulty.
						match self.get_solution(&mut stream, &mut stream_err) {
							Some(a_block_header) => {
								// Got a solution
								if a_block_header.height == b.header.height {
									b.header = a_block_header;
									info!(LOGGER,
									      "(Server ID: {}) Found valid proof of work, adding block {}",
									      self.debug_output_id,
									      b.hash());
									let res =
										self.miner.chain.process_block(b.clone(), chain::NONE);
									if let Err(e) = res {
										error!(
											LOGGER,
											"(Server ID: {}) Error validating mined block: {:?}",
											self.debug_output_id,
											e
										);
									}
									debug!(
										LOGGER,
										"(Server ID: {}) Resetting key_id in miner to None",
										self.debug_output_id
									);
								} else {
									warn!(LOGGER,
									      "(Server ID: {}) Found POW for block at height: {} -  but too late",
									      self.debug_output_id,
									      b.header.height);
								}
							}
							None => {} /* No solutions found yet, let the client keep mining the
							            * same block */
						} // checking for solution
		// sleep before restarting loop
						thread::sleep(Duration::from_millis(500));
					} // loop forever
				} // end OK() match stream
			} // match stream
		} // for stream incoming
	} // fn run_loop()
} // StratumServer
