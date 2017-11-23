// Copyright 2016 The Grin Developers
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

//! Synchronization of the local blockchain with the rest of the network. Used
//! either on a brand new node or when a node is late based on others' heads.
//! Always starts by downloading the header chain before asking either for full
//! blocks or a full UTXO set with related information.

/// How many block bodies to download in parallel
const MAX_BODY_DOWNLOADS: usize = 8;

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use core::core::hash::{Hash, Hashed};
use chain;
use p2p;
use types::Error;
use util::LOGGER;

#[derive(Debug)]
struct BlockDownload {
	hash: Hash,
	start_time: Instant,
	retries: u8,
}

/// Manages syncing the local chain with other peers. Needs both a head chain
/// and a full block chain to operate. First tries to advance the header
/// chain as much as possible, then downloads the full blocks by batches.
pub struct Syncer {
	chain: Arc<chain::Chain>,
	p2p: Arc<p2p::Server>,

	sync: Mutex<bool>,
	last_header_req: Mutex<Instant>,
	blocks_to_download: Mutex<Vec<Hash>>,
	blocks_downloading: Mutex<Vec<BlockDownload>>,
}

impl Syncer {
	pub fn new(chain_ref: Arc<chain::Chain>, p2p: Arc<p2p::Server>) -> Syncer {
		Syncer {
			chain: chain_ref,
			p2p: p2p,
			sync: Mutex::new(true),
			last_header_req: Mutex::new(Instant::now() - Duration::from_secs(2)),
			blocks_to_download: Mutex::new(vec![]),
			blocks_downloading: Mutex::new(vec![]),
		}
	}

	pub fn syncing(&self) -> bool {
		*self.sync.lock().unwrap()
	}

	/// Checks the local chain state, comparing it with our peers and triggers
	/// syncing if required.
	pub fn run(&self) -> Result<(), Error> {
		info!(LOGGER, "Sync: starting sync");

		// Loop for 10s waiting for some peers to potentially sync from.
		let start = Instant::now();
		loop {
			let pc = self.p2p.peer_count();
			if pc > 3 {
				break;
			}
			if Instant::now() - start > Duration::from_secs(10) {
				break;
			}
			thread::sleep(Duration::from_millis(200));
		}

		// Now check we actually have at least one peer to sync from.
		// If not then end the sync cleanly.
		if self.p2p.peer_count() == 0 {
			info!(LOGGER, "Sync: no peers to sync with, done.");

			let mut sync = self.sync.lock().unwrap();
			*sync = false;

			return Ok(())
		}

		// check if we have missing full blocks for which we already have a header
		self.init_download()?;

		// main syncing loop, requests more headers and bodies periodically as long
		// as a peer with higher difficulty exists and we're not fully caught up
		info!(LOGGER, "Sync: Starting loop.");
		loop {
			let tip = self.chain.get_header_head()?;

			// TODO do something better (like trying to get more) if we lose peers
			let peer = self.p2p.most_work_peer().expect("No peers available for sync.");
			let peer = peer.read().unwrap();
			debug!(
				LOGGER,
				"Sync: peer {} vs us {}",
				peer.info.total_difficulty,
				tip.total_difficulty
			);

			let more_headers = peer.info.total_difficulty > tip.total_difficulty;
			let more_bodies = {
				let blocks_to_download = self.blocks_to_download.lock().unwrap();
				let blocks_downloading = self.blocks_downloading.lock().unwrap();
				debug!(
					LOGGER,
					"Sync: blocks to download {}, block downloading {}",
					blocks_to_download.len(),
					blocks_downloading.len(),
				);
				blocks_to_download.len() > 0 || blocks_downloading.len() > 0
			};

			{
				let last_header_req = self.last_header_req.lock().unwrap().clone();
				if more_headers || (Instant::now() - Duration::from_secs(30) > last_header_req) {
					self.request_headers()?;
				}
			}
			if more_bodies {
				self.request_bodies();
			}
			if !more_headers && !more_bodies {
				// TODO check we haven't been lied to on the total work
				let mut sync = self.sync.lock().unwrap();
				*sync = false;
				break;
			}

			thread::sleep(Duration::from_secs(2));
		}
		info!(LOGGER, "Sync: done.");
		Ok(())
	}

	/// Checks the gap between the header chain and the full block chain and
	/// initializes the blocks_to_download structure with the missing full
	/// blocks
	fn init_download(&self) -> Result<(), Error> {
		// compare the header's head to the full one to see what we're missing
		let header_head = self.chain.get_header_head()?;
		let full_head = self.chain.head()?;
		let mut blocks_to_download = self.blocks_to_download.lock().unwrap();

		// go back the chain and insert for download all blocks we only have the
		// head for
		let mut prev_h = header_head.last_block_h;
		while prev_h != full_head.last_block_h {
			let header = self.chain.get_block_header(&prev_h)?;
			if header.height < full_head.height {
				break;
			}
			blocks_to_download.push(header.hash());
			prev_h = header.previous;
		}

		debug!(
			LOGGER,
			"Sync: Added {} full block hashes to download.",
			blocks_to_download.len()
		);
		Ok(())
	}

	/// Asks for the blocks we haven't downloaded yet and place them in the
	/// downloading structure.
	fn request_bodies(&self) {
		let mut blocks_to_download = self.blocks_to_download.lock().unwrap();
		let mut blocks_downloading = self.blocks_downloading.lock().unwrap();

		// retry blocks not downloading
		let now = Instant::now();
		for download in blocks_downloading.deref_mut() {
			let elapsed = (now - download.start_time).as_secs();
			if download.retries >= 12 {
				panic!("Failed to download required block {}", download.hash);
			}
			if download.retries < (elapsed / 5) as u8 {
				debug!(
					LOGGER,
					"Sync: Retry {} on block {}",
					download.retries,
					download.hash
				);
				self.request_block(download.hash);
				download.retries += 1;
			}
		}

		// consume hashes from blocks to download, place them in downloading and
		// request them from the network
		let mut count = 0;
		while blocks_to_download.len() > 0 && blocks_downloading.len() < MAX_BODY_DOWNLOADS {
			let h = blocks_to_download.pop().unwrap();
			self.request_block(h);
			count += 1;
			blocks_downloading.push(BlockDownload {
				hash: h,
				start_time: Instant::now(),
				retries: 0,
			});
		}
		debug!(
			LOGGER,
			"Sync: Requested {} full blocks to download, total left: {}. Current list: {:?}.",
			count,
			blocks_to_download.len(),
			blocks_downloading.deref(),
		);
	}

	/// We added a block, clean up the downloading structure
	pub fn block_received(&self, bh: Hash) {
		// just clean up the downloading list
		let mut bds = self.blocks_downloading.lock().unwrap();
		bds.iter()
			.position(|ref h| h.hash == bh)
			.map(|n| bds.remove(n));
	}

	/// Request some block headers from a peer to advance us
	fn request_headers(&self) -> Result<(), Error> {
		{
			let mut last_header_req = self.last_header_req.lock().unwrap();
			*last_header_req = Instant::now();
		}

		let tip = self.chain.get_header_head()?;
		let peer = self.p2p.most_work_peer();
		let locator = self.get_locator(&tip)?;
		if let Some(p) = peer {
			let p = p.read().unwrap();
			debug!(
				LOGGER,
				"Sync: Asking peer {} for more block headers, locator: {:?}",
				p.info.addr,
				locator,
			);
			if let Err(e) = p.send_header_request(locator) {
				debug!(LOGGER, "Sync: peer error, will retry");
			}
		} else {
			warn!(LOGGER, "Sync: Could not get most worked peer to request headers.");
		}
		Ok(())
	}

	/// We added a header, add it to the full block download list
	pub fn headers_received(&self, bhs: Vec<Hash>) {
		let mut blocks_to_download = self.blocks_to_download.lock().unwrap();
		for h in bhs {
			// enlist for full block download
			blocks_to_download.insert(0, h);
		}

		// we may still have more headers to retrieve but the main loop
		// will take care of this for us
	}

	/// Builds a vector of block hashes that should help the remote peer sending
	/// us the right block headers.
	fn get_locator(&self, tip: &chain::Tip) -> Result<Vec<Hash>, Error> {
		let heights = get_locator_heights(tip.height);

		debug!(LOGGER, "Sync: locator heights: {:?}", heights);

		let locator = heights
			.into_iter()
			.map(|h| self.chain.get_header_by_height(h))
			.filter(|h| h.is_ok())
			.map(|h| h.unwrap().hash())
			.collect();
		debug!(LOGGER, "Sync: locator: {:?}", locator);
		Ok(locator)
	}

	/// Pick a random peer and ask for a block by hash
	fn request_block(&self, h: Hash) {
		if let Some(peer) = self.p2p.random_peer() {
			let peer = peer.read().unwrap();
			if let Err(e) = peer.send_block_request(h) {
				debug!(LOGGER, "Sync: Error requesting block: {:?}", e);
			}
		}
	}
}

// current height back to 0 decreasing in powers of 2
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

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn test_get_locator_heights() {
		assert_eq!(get_locator_heights(0), vec![0]);
		assert_eq!(get_locator_heights(1), vec![1, 0]);
		assert_eq!(get_locator_heights(2), vec![2, 0]);
		assert_eq!(get_locator_heights(3), vec![3, 1, 0]);
		assert_eq!(get_locator_heights(10), vec![10, 8, 4, 0]);
		assert_eq!(get_locator_heights(100), vec![100, 98, 94, 86, 70, 38, 0]);
		assert_eq!(
			get_locator_heights(1000),
			vec![1000, 998, 994, 986, 970, 938, 874, 746, 490, 0]
		);
		// check the locator is still a manageable length, even for large numbers of headers
		assert_eq!(
			get_locator_heights(10000),
			vec![10000, 9998, 9994, 9986, 9970, 9938, 9874, 9746, 9490, 8978, 7954, 5906, 1810, 0]
		);
	}
}
