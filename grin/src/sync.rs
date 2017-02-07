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

use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, Duration};

use core::core::hash::{Hash, Hashed};
use chain;
use p2p;

pub struct Syncer {
	chain_store: Arc<chain::ChainStore>,
	p2p: Arc<p2p::Server>,

	sync: Mutex<bool>,
	last_header_req: Mutex<Instant>,
	blocks_to_download: Mutex<Vec<Hash>>,
	blocks_downloading: Mutex<Vec<(Hash, Instant)>>,
}

impl Syncer {
	pub fn new(chain_store: Arc<chain::ChainStore>, p2p: Arc<p2p::Server>) -> Syncer {
		Syncer {
			chain_store: chain_store,
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
	pub fn run(&self) -> Result<(), chain::Error> {
		debug!("Starting syncer.");
		let start = Instant::now();
		loop {
			let pc = self.p2p.peer_count();
			if pc > 3 {
				break;
			}
			if pc > 0 && (Instant::now() - start > Duration::from_secs(15)) {
				break;
			}
			thread::sleep(Duration::from_millis(200));
		}

		// check if we have missing full blocks for which we already have a header
		self.init_download()?;

		// main syncing loop, requests more headers and bodies periodically as long
		// as a peer with higher difficulty exists and we're not fully caught up
		info!("Starting sync loop.");
		loop {
			let tip = self.chain_store.get_header_head()?;
			// TODO do something better (like trying to get more) if we lose peers
			let peer = self.p2p.most_work_peer().unwrap();

			let more_headers = peer.info.total_difficulty > tip.total_difficulty;
			let more_bodies = {
				let blocks_to_download = self.blocks_to_download.lock().unwrap();
				let blocks_downloading = self.blocks_downloading.lock().unwrap();
				blocks_to_download.len() > 0 || blocks_downloading.len() > 0
			};

			{
				let last_header_req = self.last_header_req.lock().unwrap().clone();
				if more_headers && (Instant::now() - Duration::from_secs(2) > last_header_req) {
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
		info!("Sync done.");
		Ok(())
	}

	/// Checks the gap between the header chain and the full block chain and
	/// initializes the blocks_to_download structure with the missing full
	/// blocks
	fn init_download(&self) -> Result<(), chain::Error> {
		// compare the header's head to the full one to see what we're missing
		let header_head = self.chain_store.get_header_head()?;
		let full_head = self.chain_store.head()?;
		let mut blocks_to_download = self.blocks_to_download.lock().unwrap();

		// go back the chain and insert for download all blocks we only have the
		// head for
		let mut prev_h = header_head.last_block_h;
		while prev_h != full_head.last_block_h {
			let header = self.chain_store.get_block_header(&prev_h)?;
			blocks_to_download.push(header.hash());
			prev_h = header.previous;
		}

		debug!("Added {} full block hashes to download.",
		       blocks_to_download.len());
		Ok(())
	}

	/// Asks for the blocks we haven't downloaded yet and place them in the
	/// downloading structure.
	fn request_bodies(&self) {
		let mut blocks_downloading = self.blocks_downloading.lock().unwrap();
		if blocks_downloading.len() > MAX_BODY_DOWNLOADS {
			// clean up potentially dead downloads
			let twenty_sec_ago = Instant::now() - Duration::from_secs(20);
			blocks_downloading.iter()
				.position(|&h| h.1 < twenty_sec_ago)
				.map(|n| blocks_downloading.remove(n));
		} else {
			// consume hashes from blocks to download, place them in downloading and
			// request them from the network
			let mut blocks_to_download = self.blocks_to_download.lock().unwrap();
			while blocks_to_download.len() > 0 && blocks_downloading.len() < MAX_BODY_DOWNLOADS {
				let h = blocks_to_download.pop().unwrap();
				let peer = self.p2p.random_peer().unwrap();
				peer.send_block_request(h);
				blocks_downloading.push((h, Instant::now()));
			}
			debug!("Requesting more full block hashes to download, total: {}.",
			       blocks_to_download.len());
		}
	}

	/// We added a block, clean up the downloading structure
	pub fn block_received(&self, bh: Hash) {
		// just clean up the downloading list
		let mut bds = self.blocks_downloading.lock().unwrap();
		bds.iter().position(|&h| h.0 == bh).map(|n| bds.remove(n));
	}

	/// Request some block headers from a peer to advance us
	fn request_headers(&self) -> Result<(), chain::Error> {
		{
			let mut last_header_req = self.last_header_req.lock().unwrap();
			*last_header_req = Instant::now();
		}

		let tip = self.chain_store.get_header_head()?;
		let peer = self.p2p.most_work_peer();
		let locator = self.get_locator(&tip)?;
		if let Some(p) = peer {
			debug!("Asking peer {} for more block headers.", p.info.addr);
			p.send_header_request(locator)?;
		} else {
			warn!("Could not get most worked peer to request headers.");
		}
		Ok(())
	}

	/// We added a header, add it to the full block download list
	pub fn headers_received(&self, bhs: Vec<Hash>) {
		let mut blocks_to_download = self.blocks_to_download.lock().unwrap();
		let hs_len = bhs.len();
		for h in bhs {
			// enlist for full block download
			blocks_to_download.insert(0, h);
		}
		// ask for more headers if we got as many as required
		if hs_len == (p2p::MAX_BLOCK_HEADERS as usize) {
			self.request_headers();
		}
	}

	/// Builds a vector of block hashes that should help the remote peer sending
	/// us the right block headers.
	fn get_locator(&self, tip: &chain::Tip) -> Result<Vec<Hash>, chain::Error> {
		// Prepare the heights we want as the latests height minus increasing powers
		// of 2 up to max.
		let mut heights = vec![tip.height];
		let mut tail = (1..p2p::MAX_LOCATORS)
			.map(|n| 2u64.pow(n))
			.filter_map(|n| if n > tip.height {
				None
			} else {
				Some(tip.height - n)
			})
			.collect::<Vec<_>>();
		heights.append(&mut tail);

		// Iteratively travel the header chain back from our head and retain the
		// headers at the wanted heights.
		let mut header = self.chain_store.get_block_header(&tip.last_block_h)?;
		let mut locator = vec![];
		while heights.len() > 0 {
			if header.height == heights[0] {
				heights = heights[1..].to_vec();
				locator.push(header.hash());
			}
			if header.height > 0 {
				header = self.chain_store.get_block_header(&header.previous)?;
			}
		}
		Ok(locator)
	}
}
