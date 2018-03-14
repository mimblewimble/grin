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

//! Stratum Miner, Connects to a grin stratum server, Gets new blocks and
//! mines them, Returns solutions

extern crate grin_core as core;
extern crate grin_pow as pow;
extern crate grin_util as util;
extern crate rand;

use std::mem;
use std::thread;
use std::time::Duration;
use std::net::{Shutdown, TcpStream};
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::Sender;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::channel;
use rand::Rng;

use pow::cuckoo;
use pow::MiningWorker;
use core::ser;
use core::core::{Block, BlockHeader};
use core::consensus;

// Receive a block from the parent, and mine it.  Check for a new block
// occasionally.  Send solutions back to the parent.  Exit on failure,
// the parent will re-spawn.
fn run_mining_thread(
	id: usize,
	miner: &mut cuckoo::Miner,
	s: Sender<BlockHeader>,
	r: Receiver<Block>,
	cuckoo_size: u32,
) {
	let debug_output_id = String::from("Miner1");
	let mut b: Block;

	println!("(Server ID: {}) Waiting for a block....", id);

	// Blocking wait for an initial block
	loop {
		let blk = r.recv();
		if blk.is_err() {
			// I can't recover from a bad comm channel with the parent thread
			return;
		} else {
			println!("Thread {} got a block: {:?}", id, blk);
		}
		b = blk.unwrap();
		// dont mine a zero block.
		if b.header.height != 0 {
			break;
		}
	}

	println!( "(Server ID: {}) Mining at Cuckoo{} at difficulty {} at height {} with nonce: {}: run_mining_thread",
    debug_output_id,
    cuckoo_size,
    b.header.difficulty,
    b.header.height,
    b.header.nonce
  );

	loop {
		// Do mining for a bit
		for _ in 1..42 {
			b.header.nonce += 1;
			// XXX TODO:  If we wrap the nonce, +1 the timestamp
			let pow_hash = b.hash();
			if let Ok(proof) = miner.mine(&pow_hash[..]) {
				let proof_diff = proof.clone().to_difficulty();
				if proof_diff >= b.header.difficulty {
					println!(
						"Thread {} found a solution: pow={:?} nonce={} height={}",
						id, proof, b.header.nonce, b.header.height
					);
					b.header.pow = proof;
					s.send(b.header.clone()).unwrap();
				}
			}
		}
		// Check for a new block
		// println!("Thread {} checking for new block", id);
		let val = r.try_recv();
		if !val.is_err() {
			// There is a new Block
			b = val.unwrap();
			println!(
				"Thread {} got a NEW Block to mine at height {} with nonce {}",
				id, b.header.height, b.header.nonce
			);
		}
	} // loop
}

// Send a solution (BlockHeader) to the stratum server
fn stratum_send_solution(stream: &mut TcpStream, bh: BlockHeader, stream_err: &mut bool) {
	// Send the solution data to the server

	// Serialize it into a vector
	let mut hvec = Vec::new();
	ser::serialize(&mut hvec, &bh).expect("serialization failed");

	// send the size
	let hveclen: u32 = hvec.len() as u32;
	unsafe {
		let hsz = mem::transmute::<u32, [u8; 4]>(hveclen);
		match stream.write(&hsz) {
			Ok(_) => {
				println!("Sent solved block header size");
			}
			Err(e) => {
				println!("Error in connection with stratum server: {}", e);
				*stream_err = true;
				return;
			}
		}
	}

	// send the solved block header
	match stream.write(&hvec) {
		Ok(_) => {
			println!("Sent solved block header");
		}
		Err(e) => {
			println!("Error in connection with stratum server: {}", e);
			*stream_err = true;
			return;
		}
	}
}

// Get a block from the stratum server to mine
fn stratum_get_block(stream: &mut TcpStream, stream_err: &mut bool) -> Option<Block> {
	// Get the size of the next block
	let block_usz: usize;
	let mut data = [0 as u8; 4]; // using 4 byte buffer
	match stream.read_exact(&mut data) {
		Ok(_) => unsafe {
			let sz = mem::transmute::<[u8; 4], u32>(data);
			block_usz = sz as usize;
		},
		Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
			// This is not an 'error', just isnt any data ready for us to read right now
			return None;
		}
		Err(e) => {
			println!("Error in connection with stratum server: {}", e);
			*stream_err = true;
			return None;
		}
	}

	// Get serialized block data
	let mut retry = 0;
	let mut bvec = vec![0; block_usz];
	loop {
		match stream.read_exact(&mut bvec) {
			Ok(_) => {
				break;
			}
			Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
				// Sleep a bit and try again, but give up at some point
				retry += 1;
				if retry >= 1000 {
					println!("Error in connection with stratum server: {}", e);
					*stream_err = true;
					return None;
				}
				thread::sleep(Duration::from_millis(100));
			}
			Err(e) => {
				println!("Error in connection with stratum server: {}", e);
			}
		}
		{}
	}
	let b: Block = ser::deserialize(&mut &bvec[..]).unwrap();
	return Some(b);
}

fn main() {
	// Set some defaults
	let num_threads: usize = 6;
	let cuckoo_size: u32 = 16;
	let proof_size: usize = 42;

	// Vectors to hold Threads and communication Channels
	let mut threads = vec![];
	let mut sendc = vec![];
	let mut recvc = vec![];

	// Create (degenerate) thread Channels just to init the vectors
	for _t_num in 0..num_threads {
		let (_tsend, mrecv) = channel();
		let (msend, _trecv) = channel();
		recvc.push(mrecv);
		sendc.push(msend);
	}

	// To generate new nonces
	let mut rng = rand::OsRng::new().unwrap();

	let mut stream_err: bool;

	// Main Loop
	loop {
		// Connect to the server
		match TcpStream::connect("localhost:13416") {
			Ok(mut stream) => {
				println!("Successfully connected to server in port 13416");
				stream
					.set_nonblocking(true)
					.expect("set_nonblocking call failed");
				stream_err = false;

				// Server Handshake XXX TODO
				// Send Wallet URL
				// Send requested initial difficulty
				// More?

				// A block will be recvd from the server and sent to each mining thread
				let mut block_to_mine: Block = Block::default();

				// Working Loop
				loop {
					// Abort connection on error
					if stream_err == true {
						println!("Miner detected stream error");
						match stream.shutdown(Shutdown::Both) {
							_ => {}
						}
						break;
					}

					// Non-Blocking check for result
					// (Re)Start threads as needed
					// println!("Check for worker results");
					for t_num in 0..num_threads {
						let val = recvc[t_num].try_recv();
						if !val.is_err() {
							// There was a Block solution waiting for us, we got it in val
							let bh: BlockHeader = val.unwrap();
							println!(
								"Main got a BlockHeader Solution From Thread {}: {:?}",
								t_num, bh
							);

							let mut hvec = Vec::new();
							ser::serialize(&mut hvec, &bh)
								.expect("serialization of BlockHeader result failed");

							// Send this solved block to the server
							println!("Return a solved block header:");
							println!("{:?}", bh.height);
							println!("{:?}", bh.pow);
							println!("{:?}", bh.nonce);
							stratum_send_solution(&mut stream, bh, &mut stream_err);
						} else {
							match val {
								Err(TryRecvError::Disconnected) => {
									println!(
										"Thread Communication error - Restarting thread: {}",
										t_num
									);
									// Create a miner to do our mining
									let mut miner = cuckoo::Miner::new(
										consensus::EASINESS,
										cuckoo_size,
										proof_size,
									);
									let (tsend, mrecv) = channel();
									let (msend, trecv) = channel();
									sendc[t_num] = msend;
									recvc[t_num] = mrecv;
									let th = thread::spawn(move || {
										run_mining_thread(
											t_num,
											&mut miner,
											tsend,
											trecv,
											cuckoo_size,
										);
									});
									// UGLY HACK
									if threads.len() < num_threads {
										threads.push(th);
									} else {
										threads[t_num] = th;
									}
									// Send a copy of the Block to the thread
									block_to_mine.header.nonce = rng.gen();
									// XXX TODO: Randomize the timestamp a bit also
									sendc[t_num].send(block_to_mine.clone()).unwrap();
								}
								_ => {}
							};
						}
					}

					// non-blocking check for new block
					match stratum_get_block(&mut stream, &mut stream_err) {
						None => {}
						Some(a_block) => {
							block_to_mine = a_block;
							// Send it to all threads
							println!("Main thread Got a new block at heaight {} - sending to the threads", block_to_mine.header.height);
							for t_num in 0..num_threads {
								// Set a unique nonce for each thread
								block_to_mine.header.nonce = rng.gen();
								// XXX TODO: Randomize the timestamp a bit also
								sendc[t_num].send(block_to_mine.clone()).unwrap();
							}
						}
					}

					// sleep before restarting loop
					thread::sleep(Duration::from_millis(500));
				} // Working loop
			}
			Err(e) => {
				println!("Main thread Connection Attempt with server failed: {}", e);
			}
		}
		println!("Will try again in 5 seconds");
		thread::sleep(Duration::from_millis(5000));
	} // Main loop
}
