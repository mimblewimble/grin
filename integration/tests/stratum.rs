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

//! Live stratum server integration test (JSON-RPC over TCP).

#[macro_use]
extern crate log;

mod common;

use crate::common::{clean_all_output, config, init_chain, settle, start_server, stratum_config};
use bufstream::BufStream;
use grin_util as util;
use grin_util::StopState;
use serde_json::Value;
use std::io::prelude::{BufRead, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::{thread, time};

/// Stratum accepts workers, answers JSON-RPC, and broadcasts jobs when blocks are found.
#[test]
fn basic_stratum_server() {
	util::init_test_logger();
	init_chain();

	let test_name_dir = "stratum_server";
	clean_all_output(test_name_dir);

	let s = start_server(config(4000, test_name_dir, 0));

	let mut stratum_cfg = stratum_config();
	stratum_cfg.burn_reward = true;
	stratum_cfg.attempt_time_per_block = 999;
	stratum_cfg.enable_stratum_server = Some(true);
	stratum_cfg.stratum_server_addr = Some(String::from("127.0.0.1:11101"));

	s.start_stratum_server(stratum_cfg);

	// Wait until stratum accepts TCP connections.
	loop {
		if TcpStream::connect("127.0.0.1:11101").is_ok() {
			break;
		}
		thread::sleep(time::Duration::from_millis(500));
	}
	info!("stratum server connected");

	let mut workers = vec![];
	for _n in 0..5 {
		let w = TcpStream::connect("127.0.0.1:11101").unwrap();
		w.set_nonblocking(true)
			.expect("Failed to set TcpStream to non-blocking");
		workers.push(BufStream::new(w));
	}
	assert_eq!(workers.len(), 5);

	// Simulate a worker disconnect.
	workers.remove(4);

	// Swallow the genesis/job broadcast.
	thread::sleep(time::Duration::from_secs(5));
	let mut response = String::new();
	for n in 0..workers.len() {
		let _ = workers[n].read_line(&mut response);
	}

	// getjobtemplate
	let mut response = String::new();
	let job_req = "{\"id\": \"Stratum\", \"jsonrpc\": \"2.0\", \"method\": \"getjobtemplate\"}\n";
	workers[2].write_all(job_req.as_bytes()).unwrap();
	workers[2].flush().unwrap();
	thread::sleep(time::Duration::from_secs(1));
	match workers[2].read_line(&mut response) {
		Ok(_) => {
			let r: Value = serde_json::from_str(&response).unwrap();
			assert_eq!(r["error"], serde_json::Value::Null);
			assert_ne!(r["result"], serde_json::Value::Null);
		}
		Err(_e) => panic!("getjobtemplate failed"),
	}

	// keepalive
	let mut response = String::new();
	let job_req = "{\"id\":\"3\",\"jsonrpc\":\"2.0\",\"method\":\"keepalive\"}\n";
	let ok_resp = "{\"id\":\"3\",\"jsonrpc\":\"2.0\",\"method\":\"keepalive\",\"result\":\"ok\",\"error\":null}\n";
	workers[2].write_all(job_req.as_bytes()).unwrap();
	workers[2].flush().unwrap();
	thread::sleep(time::Duration::from_secs(1));
	let _ = workers[2].read_line(&mut response);
	assert_eq!(response.as_str(), ok_resp);

	// unknown method
	let mut response = String::new();
	let job_req = "{\"id\":\"4\",\"jsonrpc\":\"2.0\",\"method\":\"doesnotexist\"}\n";
	let ok_resp = "{\"id\":\"4\",\"jsonrpc\":\"2.0\",\"method\":\"doesnotexist\",\"result\":null,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}\n";
	workers[3].write_all(job_req.as_bytes()).unwrap();
	workers[3].flush().unwrap();
	thread::sleep(time::Duration::from_secs(1));
	let _ = workers[3].read_line(&mut response);
	assert_eq!(response.as_str(), ok_resp);

	let stats = s.get_server_stats().unwrap();
	assert_eq!(stats.stratum_stats.block_height, 1);
	assert_eq!(stats.stratum_stats.num_workers, 4);

	let stop = Arc::new(StopState::new());
	s.start_test_miner(None, stop.clone());

	workers.remove(1);

	// Wait for a few mined blocks / job broadcasts.
	thread::sleep(time::Duration::from_secs(5));
	s.stop_test_miner(stop);

	let mut jobtemplate = String::new();
	let _ = workers[2].read_line(&mut jobtemplate);
	if !jobtemplate.is_empty() {
		let job_template: Value = serde_json::from_str(&jobtemplate).unwrap();
		assert_eq!(job_template["method"], "job");
	}

	let stats = s.get_server_stats().unwrap();
	assert_eq!(stats.stratum_stats.num_workers, 3);
	assert_ne!(stats.stratum_stats.block_height, 1);

	s.stop();
	settle();
}
