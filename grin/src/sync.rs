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
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::net::SocketAddr;

use core::core::hash::{Hash, Hashed};
use core::core::BlockHeader;
use core::core::target::Difficulty;
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

/// Hold information about peers we've chosen to pair with for sync
/// Mostly ubset of block header with hash included
#[derive(Debug)]
struct BuddyInfo {
	/// Height of header
	pub height: u64,
	/// Hash of header
	pub hash: Hash,
	/// Total reported accumulated difficulty
	pub difficulty: Difficulty,
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

	//keep a map of the last header successfully received
	//from each peerj
	last_headers_from_peers: Mutex<HashMap<SocketAddr, BuddyInfo>>,
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
			last_headers_from_peers: Mutex::new(HashMap::new()),
		}
	}

	pub fn syncing(&self) -> bool {
		*self.sync.lock().unwrap()
	}


	// Tries to select the preferred peer to request headers
	//from
	fn select_buddy(&self, current_sync_buddy: Option<Arc<RwLock<p2p::Peer>>>)
	-> Option<Arc<RwLock<p2p::Peer>>> {
		//how much the work on another node should differ before we consider
		//swapping sync buddies
		let buddy_swap_threshold = 10_000;

		//No peers at all.. problem
		let most_work_peer = self.p2p.most_work_peer();
		if let None = most_work_peer {
			return None;
		};

		//If no current buddy, simply select peer with longest TD
		if let None = current_sync_buddy {
			return most_work_peer;
		}
		
		let current_buddy_raw = current_sync_buddy.as_ref().unwrap().read().unwrap();
		let most_work_raw = most_work_peer.as_ref().unwrap().read().unwrap();
		let current_buddy_td = current_buddy_raw.info.total_difficulty.into_num();
		let most_work_td = most_work_raw.info.total_difficulty.into_num();

		let td_difference = most_work_td-current_buddy_td;
		if td_difference > buddy_swap_threshold {
			info!(LOGGER, "Swapping sync buddy to: {}", most_work_raw.info.addr);
			most_work_peer.clone()
		} else {
			current_sync_buddy.clone()
		}
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

		//Keep track of which peer we're currently syncing with
		let current_sync_buddy = None; 

		//keep a map of the latest header each peer has given us
		//(so we can swap from one to the next while still
		//providing a relevant set of locators
		//let last_headers_from_peers = HashMap::new();

		// as a peer with higher difficulty exists and we're not fully caught up
		info!(LOGGER, "Sync: Starting loop.");
		loop {
			let tip = self.chain.get_header_head()?;

			// select the best peer to be peering with
			let current_sync_buddy = self.select_buddy(current_sync_buddy.clone());
			if let None = current_sync_buddy.clone(){
				error!(LOGGER, "No peers to sync headers with. Done.");
				break;
			}

			let current_sync_buddy = current_sync_buddy.unwrap();
			let peer = current_sync_buddy.read().unwrap();

			trace!(LOGGER, "Buddy peer is: {:?}",peer.info);
			debug!(
				LOGGER,
				"Sync: buddy peer {} vs us {}",
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
			trace!(LOGGER, "Requesting headers.");
			{
				let last_header_req = self.last_header_req.lock().unwrap().clone();
				if more_headers || (Instant::now() - Duration::from_secs(30) > last_header_req) {
					self.request_headers(&peer)?;
				}
			}
			trace!(LOGGER, "Requesting bodies.");
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
	fn request_headers(&self, peer: &p2p::Peer) -> Result<(), Error> {
		{
			let mut last_header_req = self.last_header_req.lock().unwrap();
			*last_header_req = Instant::now();
		}
		//Get the last header sent by current sync buddy, which we'll use to build locators
		{
			let last_headers_from_peers = self.last_headers_from_peers.lock().unwrap();
			let buddy_info = last_headers_from_peers.get(&peer.info.addr);
			
			let locator = self.get_locator(buddy_info)?;
			debug!(
				LOGGER,
				"Sync: Asking peer {} for more block headers, locator: {:?}",
				peer.info.addr,
				locator,
			);
			if let Err(_) = peer.send_header_request(locator) {
				debug!(LOGGER, "Sync: peer error, will retry");
			}
		}
		Ok(())
	}

	/// We added a header, add it to the full block download list
	pub fn headers_received(&self, bhs: Vec<BlockHeader>, last_received_height: u64, addr: SocketAddr) {
		debug!(LOGGER, "Headers received from : {}", addr);
		let mut blocks_to_download = self.blocks_to_download.lock().unwrap();
		for h in &bhs {
			// enlist for full block download
			blocks_to_download.insert(0, h.hash());
		}

		let last_stored_height = {
			let last_headers_from_peers = self.last_headers_from_peers.lock().unwrap();
			let last_stored_info = last_headers_from_peers.get(&addr);
			 match last_stored_info {
				None => 0,
				Some(h) => h.height,
			}
		};
		if bhs.len()==0 {
			return;
		}
		let mut last_headers_from_peers = self.last_headers_from_peers.lock().unwrap();
		if last_received_height > last_stored_height {
			let new_header = bhs.last().unwrap();
			last_headers_from_peers.insert(addr, BuddyInfo {
				hash: new_header.hash(),
				difficulty: new_header.difficulty.clone(),
				height: last_received_height
			});
		}

		// we may still have more headers to retrieve but the main loop
		// will take care of this for us
	}

	/// Builds a vector of block hashes that should help the remote peer sending
	/// us the right block headers.
	/// Note height isn't necessarily our height, it can be the height reported
	/// by a peer
	fn get_locator(&self, buddy_tip: Option<&BuddyInfo>) -> Result<Vec<Hash>, Error> {
		// Prepare the heights we want as the latests height minus increasing powers
		// of 2 up to max.
		let buddy_tip_height = match buddy_tip {
			Some(t) => t.height,
			None => 0,
		};
		let mut heights = vec![buddy_tip_height];
		let mut tail = (1..p2p::MAX_LOCATORS)
			.map(|n| 2u64.pow(n))
			.filter_map(|n| if n > buddy_tip_height {
				None
			} else {
				Some(buddy_tip_height - n)
			})
			.collect::<Vec<_>>();
		heights.append(&mut tail);

		// Include the genesis block (height 0) here as a fallback to guarantee
		// both nodes share at least one common header hash in the locator
		heights.push(0);

		debug!(LOGGER, "Sync: Locator heights (from sync buddy): {:?}", heights);
		if !buddy_tip.is_some() {
			//just return genesis (cause we have no buddy)
			return Ok(vec![self.chain.get_header_by_height(0).unwrap().hash()]);
		}

		// Iteratively travel the header chain back from head and retain the
		// headers at the wanted heights.
		let header = self.chain.get_block_header(&buddy_tip.unwrap().hash);
		if let Err(_) = header.as_ref()  {
			//Whatever we have from our buddy doesn't match what we're expecting
			//So start over again to find a common block
			return Ok(vec![self.chain.get_header_by_height(0).unwrap().hash()]);
			//TODO: Check if below is what we should be doing, starting from our
			//own expectation of what the header should be
			/*header = self.chain.get_block_header(
				&self.chain.get_header_head().unwrap().last_block_h);*/
		}
		let mut header = header.unwrap();
		let mut locator = vec![];
		while heights.len() > 0 {
			if header.height == heights[0] {
				heights = heights[1..].to_vec();
				locator.push(header.hash());
			}
			if header.height > 0 {
				header = self.chain.get_block_header(&header.previous)?;
			}
		}
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
