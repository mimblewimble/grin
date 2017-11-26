// Copyright 2017 The Grin Developers
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

use std::thread;
use std::time::Duration;
use std::sync::{Arc, RwLock};

use chain;
use core::core::hash::{Hash, Hashed};
use p2p;
use p2p::Peer;
use types::Error;
use util::LOGGER;

pub fn run_block_sync(
	adapter: Arc<p2p::NetAdapter>,
	p2p_server: Arc<p2p::Server>,
	chain: Arc<chain::Chain>,
) {
	let _ = thread::Builder::new()
		.name("header_sync".to_string())
		.spawn(move || {
			loop {
				debug!(LOGGER, "block_sync: loop");

				let header_head = chain.get_header_head().unwrap();
				let block_header = chain.head_header().unwrap();
				let mut hashes = vec![];

				if header_head.total_difficulty > block_header.total_difficulty {
					let mut current = chain.get_block_header(&header_head.last_block_h);
					while let Ok(header) = current {
						if header.hash() == block_header.hash() {
							break;
						}
						hashes.push(header.hash());
						current = chain.get_block_header(&header.previous);
					}
				}
				hashes.reverse();

				let hashes_to_get = hashes
					.iter()
					.filter(|x| !chain.get_block(&x).is_ok())
					.take(10)
					.cloned()
					.collect::<Vec<_>>();

				if hashes_to_get.len() > 0 {
					debug!(
						LOGGER,
						"block_sync: requesting blocks ({}/{}), {:?}",
						block_header.height,
						header_head.height,
						hashes_to_get,
					);

					for hash in hashes_to_get.clone() {
						let peer = if hashes_to_get.len() < 100 {
							p2p_server.most_work_peer()
						} else {
							p2p_server.random_peer()
						};
						if let Some(peer) = peer {
							let peer = peer.read().unwrap();
							if let Err(e) = peer.send_block_request(hash) {
								debug!(LOGGER, "block_sync: error requesting block: {:?}, {:?}", hash, e);
							}
						}
					}
					thread::sleep(Duration::from_secs(1));
				} else {
					thread::sleep(Duration::from_secs(5));
				}
			}
		});
}

pub fn run_header_sync(
	adapter: Arc<p2p::NetAdapter>,
	p2p_server: Arc<p2p::Server>,
	chain: Arc<chain::Chain>,
) {
	let adapter = adapter.clone();
	let _ = thread::Builder::new()
		.name("header_sync".to_string())
		.spawn(move || {
			loop {
				debug!(LOGGER, "header_sync: loop");

				let difficulty = adapter.total_difficulty();

				if let Some(peer) = p2p_server.most_work_peer() {
					let peer = peer.clone();
					let p = peer.read().unwrap();
					let peer_difficulty = p.info.total_difficulty.clone();

					if peer_difficulty > difficulty {
						debug!(
							LOGGER,
							"header_sync: difficulty {} vs {}",
							peer_difficulty,
							difficulty,
						);

						let _ = request_headers(
							peer.clone(),
							p2p_server.clone(),
							chain.clone(),
						);
					}

				}

				thread::sleep(Duration::from_secs(30));
			}
		});
}

/// Request some block headers from a peer to advance us
fn request_headers(
	peer: Arc<RwLock<Peer>>,
	p2p_server: Arc<p2p::Server>,
	chain: Arc<chain::Chain>,
) -> Result<(), Error> {
	let locator = get_locator(chain)?;
	let peer = peer.read().unwrap();
	debug!(
		LOGGER,
		"Sync: Asking peer {} for more block headers, locator: {:?}",
		peer.info.addr,
		locator,
	);
	let _ = peer.send_header_request(locator);
	Ok(())
}

fn get_locator(chain: Arc<chain::Chain>) -> Result<Vec<Hash>, Error> {
	let tip = chain.get_header_head()?;

	// go back to earlier header height to ensure we do not miss a header
	let height = if tip.height > 10 {
		tip.height - 10
	} else {
		0
	};
	let heights = get_locator_heights(height);

	debug!(LOGGER, "Sync: locator heights: {:?}", heights);

	let mut locator = vec![];
	let mut current = chain.get_block_header(&tip.last_block_h);
	while let Ok(header) = current {
		if heights.contains(&header.height) {
			locator.push(header.hash());
		}
		current = chain.get_block_header(&header.previous);
	}

	debug!(LOGGER, "Sync: locator: {:?}", locator);

	Ok(locator)
}

fn get_locator_heights(height: u64) -> Vec<u64> {
	let mut current = height.clone();
	let mut heights = vec![];
	while current > 0 {
		heights.push(current);
		let next = 2u64.pow(heights.len() as u32);
		current = if current > next {
			current - next
		} else {
			0
		}
	}
	heights.push(0);
	heights
}
