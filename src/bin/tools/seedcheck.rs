// Copyright 2024 The Grin Developers
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

/// Relatively self-contained seed health checker
use std::sync::Arc;

use grin_core::core::hash::Hashed;
use grin_core::pow::Difficulty;
use grin_core::{genesis, global};
use grin_p2p as p2p;
use grin_servers::{resolve_dns_to_addrs, MAINNET_DNS_SEEDS, TESTNET_DNS_SEEDS};
use p2p::types::{MAINNET_PEER_PORT, TESTNET_PEER_PORT};
use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SeedCheckError {
	#[error("Seed Connect Error {0}")]
	SeedConnectError(String),
	#[error("Grin Store Error {0}")]
	StoreError(String),
}

impl From<p2p::Error> for SeedCheckError {
	fn from(e: p2p::Error) -> Self {
		SeedCheckError::SeedConnectError(format!("{:?}", e))
	}
}

impl From<grin_store::lmdb::Error> for SeedCheckError {
	fn from(e: grin_store::lmdb::Error) -> Self {
		SeedCheckError::StoreError(format!("{:?}", e))
	}
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SeedCheckResults {
	pub mainnet: Vec<SeedCheckResult>,
	pub testnet: Vec<SeedCheckResult>,
}

impl Default for SeedCheckResults {
	fn default() -> Self {
		Self {
			mainnet: vec![],
			testnet: vec![],
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeedCheckResult {
	pub url: String,
	pub dns_resolutions_found: bool,
	pub success: bool,
	pub successful_attempts: Vec<SeedCheckConnectAttempt>,
	pub unsuccessful_attempts: Vec<SeedCheckConnectAttempt>,
}

impl Default for SeedCheckResult {
	fn default() -> Self {
		Self {
			url: "".into(),
			dns_resolutions_found: false,
			success: false,
			successful_attempts: vec![],
			unsuccessful_attempts: vec![],
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeedCheckConnectAttempt {
	pub ip_addr: String,
	pub handshake_success: bool,
	pub user_agent: Option<String>,
	pub capabilities: Option<String>,
	pub error: Option<String>,
}

pub fn check_seeds(is_testnet: bool, seed: Option<&str>) -> Vec<SeedCheckResult> {
	let mut result = vec![];
	let (default_seeds, port) = match is_testnet {
		true => (TESTNET_DNS_SEEDS, TESTNET_PEER_PORT),
		false => (MAINNET_DNS_SEEDS, MAINNET_PEER_PORT),
	};
	let seeds = match seed {
		Some(seed) => vec![seed],
		None => default_seeds.to_vec(),
	};

	if is_testnet {
		global::set_local_chain_type(global::ChainTypes::Testnet);
	}

	eprintln!(
		"Running seedcheck for {} on port {}",
		if is_testnet { "testnet" } else { "mainnet" },
		port
	);

	let config = p2p::types::P2PConfig::default();
	let adapter = Arc::new(p2p::DummyAdapter {});
	let tmp_root = ".__grintmp__";
	let mut data_root = PathBuf::from(tmp_root);
	data_root.push(format!("seedcheck-{}", std::process::id()));
	let peer_store_root = data_root.join("peer_store_root");
	let peers = Arc::new(p2p::Peers::new(
		p2p::store::PeerStore::new(&peer_store_root.to_string_lossy()).unwrap(),
		adapter,
		config.clone(),
	));

	for s in seeds.iter() {
		info!("Checking seed health for {}", s);
		eprintln!("Checking seed {}", s);
		let mut seed_result = SeedCheckResult::default();
		seed_result.url = s.to_string();
		let resolved_dns_entries = resolve_dns_to_addrs(&vec![format!("{}:{}", s, port)]);
		if resolved_dns_entries.is_empty() {
			info!("FAIL - No dns entries found for {}", s);
			result.push(seed_result);
			continue;
		}
		seed_result.dns_resolutions_found = true;
		// Check backwards, last contains the latest (at least on my machine!)
		for r in resolved_dns_entries.iter().rev() {
			let res = check_seed_health(*r, is_testnet, &peers);
			if let Ok(p) = res {
				let user_agent = p.info.user_agent.clone();
				let capabilities = format!("{:?}", p.info.capabilities);
				info!(
					"SUCCESS - Performed Handshake with seed for {} at {}. {} - {:?}",
					s, r, user_agent, p.info.capabilities
				);
				p.stop();
				p.wait();
				//info!("{:?}", p);
				seed_result.success = true;
				seed_result
					.successful_attempts
					.push(SeedCheckConnectAttempt {
						ip_addr: r.to_string(),
						handshake_success: true,
						user_agent: Some(user_agent),
						capabilities: Some(capabilities),
						error: None,
					});
			} else if let Err(e) = res {
				seed_result
					.unsuccessful_attempts
					.push(SeedCheckConnectAttempt {
						ip_addr: r.to_string(),
						handshake_success: false,
						user_agent: None,
						capabilities: None,
						error: Some(e.to_string()),
					});
			}
		}

		if !seed_result.success {
			info!(
				"FAIL - Unable to handshake at any known DNS resolutions for {}",
				s
			);
		}

		result.push(seed_result);
	}

	drop(peers);

	// Clean up temporary files for this process, then remove the common root
	// only if no other seedcheck process is using it.
	if let Err(e) = fs::remove_dir_all(&data_root) {
		debug!("Unable to delete temporary seedcheck files: {:?}", e);
		eprintln!(
			"WARN cleanup: unable to delete temporary seedcheck files: {:?}",
			e
		);
	}
	let _ = fs::remove_dir(tmp_root);

	result
}

fn check_seed_health(
	addr: p2p::PeerAddr,
	is_testnet: bool,
	peers: &Arc<p2p::Peers>,
) -> Result<p2p::Peer, SeedCheckError> {
	let config = p2p::types::P2PConfig::default();
	let capabilities = p2p::types::Capabilities::default();
	let genesis_hash = match is_testnet {
		true => genesis::genesis_test().hash(),
		false => genesis::genesis_main().hash(),
	};

	let handshake = p2p::handshake::Handshake::new(genesis_hash, config.clone());

	match TcpStream::connect_timeout(&addr.0, Duration::from_secs(5)) {
		Ok(stream) => {
			let self_addr = p2p::PeerAddr::from_ip(config.host);
			let total_diff = Difficulty::from_num(1);

			let peer = p2p::Peer::connect(
				stream,
				capabilities,
				total_diff,
				self_addr,
				&handshake,
				peers.clone(),
			)?;
			Ok(peer)
		}
		Err(e) => {
			trace!(
				"connect_peer: on {}:{}. Could not connect to {}: {:?}",
				config.host,
				config.port,
				addr,
				e
			);
			Err(p2p::Error::Connection(e).into())
		}
	}
}
