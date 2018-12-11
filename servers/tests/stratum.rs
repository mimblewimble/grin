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

#[macro_use]
extern crate log;

mod framework;

use self::core::global::{self, ChainTypes};
use crate::framework::{config, stratum_config};
use bufstream::BufStream;
use grin_core as core;
use grin_servers as servers;
use grin_util as util;
use grin_util::{Mutex, StopState};
use serde_json::Value;
use std::io::prelude::{BufRead, Write};
use std::net::TcpStream;
use std::process;
use std::sync::Arc;
use std::{thread, time};

// Create a grin server, and a stratum server.
// Simulate a few JSONRpc requests and verify the results.
// Validate disconnected workers
// Validate broadcasting new jobs
#[test]
fn basic_stratum_server() {
	util::init_test_logger();
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "stratum_server";
	framework::clean_all_output(test_name_dir);

	// Create a server
	let s = servers::Server::new(config(4000, test_name_dir, 0)).unwrap();

	// Get mining config with stratumserver enabled
	let mut stratum_cfg = stratum_config();
	stratum_cfg.burn_reward = true;
	stratum_cfg.attempt_time_per_block = 999;
	stratum_cfg.enable_stratum_server = Some(true);
	stratum_cfg.stratum_server_addr = Some(String::from("127.0.0.1:11101"));

	// Start stratum server
	s.start_stratum_server(stratum_cfg);

	// Wait for stratum server to start and
	// Verify stratum server accepts connections
	loop {
		if let Ok(_stream) = TcpStream::connect("127.0.0.1:11101") {
			break;
		} else {
			thread::sleep(time::Duration::from_millis(500));
		}
		// As this stream falls out of scope it will be disconnected
	}
	info!("stratum server connected");

	// Create a few new worker connections
	let mut workers = vec![];
	for _n in 0..5 {
		let w = TcpStream::connect("127.0.0.1:11101").unwrap();
		w.set_nonblocking(true)
			.expect("Failed to set TcpStream to non-blocking");
		let stream = BufStream::new(w);
		workers.push(stream);
	}
	assert!(workers.len() == 5);
	info!("workers length verification ok");

	// Simulate a worker lost connection
	workers.remove(4);

	// Swallow the genesis block
	thread::sleep(time::Duration::from_secs(5)); // Wait for the server to broadcast
	let mut response = String::new();
	for n in 0..workers.len() {
		let _result = workers[n].read_line(&mut response);
	}

	// Verify a few stratum JSONRpc commands
	// getjobtemplate - expected block template result
	let mut response = String::new();
	let job_req = "{\"id\": \"Stratum\", \"jsonrpc\": \"2.0\", \"method\": \"getjobtemplate\"}\n";
	workers[2].write(job_req.as_bytes()).unwrap();
	workers[2].flush().unwrap();
	thread::sleep(time::Duration::from_secs(1)); // Wait for the server to reply
	match workers[2].read_line(&mut response) {
		Ok(_) => {
			let r: Value = serde_json::from_str(&response).unwrap();
			assert_eq!(r["error"], serde_json::Value::Null);
			assert_ne!(r["result"], serde_json::Value::Null);
		}
		Err(_e) => {
			assert!(false);
		}
	}
	info!("a few stratum JSONRpc commands verification ok");

	// keepalive - expected "ok" result
	let mut response = String::new();
	let job_req = "{\"id\":\"3\",\"jsonrpc\":\"2.0\",\"method\":\"keepalive\"}\n";
	let ok_resp = "{\"id\":\"3\",\"jsonrpc\":\"2.0\",\"method\":\"keepalive\",\"result\":\"ok\",\"error\":null}\n";
	workers[2].write(job_req.as_bytes()).unwrap();
	workers[2].flush().unwrap();
	thread::sleep(time::Duration::from_secs(1)); // Wait for the server to reply
	let _st = workers[2].read_line(&mut response);
	assert_eq!(response.as_str(), ok_resp);
	info!("keepalive test ok");

	// "doesnotexist" - error expected
	let mut response = String::new();
	let job_req = "{\"id\":\"4\",\"jsonrpc\":\"2.0\",\"method\":\"doesnotexist\"}\n";
	let ok_resp = "{\"id\":\"4\",\"jsonrpc\":\"2.0\",\"method\":\"doesnotexist\",\"result\":null,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}\n";
	workers[3].write(job_req.as_bytes()).unwrap();
	workers[3].flush().unwrap();
	thread::sleep(time::Duration::from_secs(1)); // Wait for the server to reply
	let _st = workers[3].read_line(&mut response);
	assert_eq!(response.as_str(), ok_resp);
	info!("worker doesnotexist test ok");

	// Verify stratum server and worker stats
	let stats = s.get_server_stats().unwrap();
	assert_eq!(stats.stratum_stats.block_height, 1); // just 1 genesis block
	assert_eq!(stats.stratum_stats.num_workers, 4); // 5 - 1 = 4
	assert_eq!(stats.stratum_stats.worker_stats[5].is_connected, false); // worker was removed
	assert_eq!(stats.stratum_stats.worker_stats[1].is_connected, true);
	info!("stratum server and worker stats verification ok");

	// Start mining blocks
	let stop = Arc::new(Mutex::new(StopState::new()));
	s.start_test_miner(None, stop.clone());
	info!("test miner started");

	// This test is supposed to complete in 3 seconds,
	// so let's set a timeout on 10s to avoid infinite waiting happened in Travis-CI.
	let _handler = thread::spawn(|| {
		thread::sleep(time::Duration::from_secs(10));
		error!("basic_stratum_server test fail on timeout!");
		thread::sleep(time::Duration::from_millis(100));
		process::exit(1);
	});

	// Simulate a worker lost connection
	workers.remove(1);

	// Wait for a few mined blocks
	thread::sleep(time::Duration::from_secs(3));
	s.stop_test_miner(stop);

	// Verify blocks are being broadcast to workers
	let expected = String::from("job");
	let mut jobtemplate = String::new();
	let _st = workers[2].read_line(&mut jobtemplate);
	let job_template: Value = serde_json::from_str(&jobtemplate).unwrap();
	assert_eq!(job_template["method"], expected);
	info!("blocks broadcasting to workers test ok");

	// Verify stratum server and worker stats
	let stats = s.get_server_stats().unwrap();
	assert_eq!(stats.stratum_stats.num_workers, 3); // 5 - 2 = 3
	assert_eq!(stats.stratum_stats.worker_stats[2].is_connected, false); // worker was removed
	assert_ne!(stats.stratum_stats.block_height, 1);
	info!("basic_stratum_server test done and ok.");
}
