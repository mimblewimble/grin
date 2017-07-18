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

//! Facade and handler for the rest of the blockchain implementation
//! and mostly the chain pipeline.

use std::sync::{Arc, Mutex};

use secp::pedersen::Commitment;

use core::core::{Block, BlockHeader, Output};
use core::core::target::Difficulty;
use core::core::hash::Hash;
use core::{consensus, genesis, pow};
use core::pow::MiningWorker;
use grin_store;
use pipe;
use store;
use types::*;



/// Helper macro to transform a Result into an Option with None in case
/// of error
macro_rules! none_err {
  ($trying:expr) => {{
    let tried = $trying;
    if let Err(_) = tried {
      return None;
    }
    tried.unwrap()
  }}
}

/// Facade to the blockchain block processing pipeline and storage. Provides
/// the current view of the UTXO set according to the chain state. Also
/// maintains locking for the pipeline to avoid conflicting processing.
pub struct Chain {
	store: Arc<ChainStore>,
	adapter: Arc<ChainAdapter>,
	head: Arc<Mutex<Tip>>,
	block_process_lock: Arc<Mutex<bool>>,
	test_mode: bool,
}

unsafe impl Sync for Chain {}
unsafe impl Send for Chain {}

impl Chain {
	/// Initializes the blockchain and returns a new Chain instance. Does a
	/// check
	/// on the current chain head to make sure it exists and creates one based
	/// on
	/// the genesis block if necessary.
	pub fn init(test_mode: bool,
	            db_root: String,
	            adapter: Arc<ChainAdapter>)
	            -> Result<Chain, Error> {
		let chain_store = store::ChainKVStore::new(db_root)?;

		// check if we have a head in store, otherwise the genesis block is it
		let head = match chain_store.head() {
			Ok(tip) => tip,
			Err(grin_store::Error::NotFoundErr) => {
				info!("No genesis block found, creating and saving one.");
				let mut gen = genesis::genesis();
				let diff = gen.header.difficulty.clone();
				let sz = if test_mode {
					consensus::TEST_SIZESHIFT
				} else {
					consensus::DEFAULT_SIZESHIFT
				};
				let mut internal_miner = pow::cuckoo::Miner::new(consensus::EASINESS, sz as u32);
				pow::pow_size(&mut internal_miner, &mut gen.header, diff, sz as u32).unwrap();
				chain_store.save_block(&gen)?;

				// saving a new tip based on genesis
				let tip = Tip::new(gen.hash());
				chain_store.save_head(&tip)?;
				info!("Saved genesis block with hash {}", gen.hash());
				tip
			}
			Err(e) => return Err(Error::StoreErr(e)),
		};

		let head = chain_store.head()?;

		Ok(Chain {
			store: Arc::new(chain_store),
			adapter: adapter,
			head: Arc::new(Mutex::new(head)),
			block_process_lock: Arc::new(Mutex::new(true)),
			test_mode: test_mode,
		})
	}

	/// Attempt to add a new block to the chain. Returns the new chain tip if it
	/// has been added to the longest chain, None if it's added to an (as of
	/// now)
	/// orphan chain.
	pub fn process_block(&self, b: &Block, opts: Options) -> Result<Option<Tip>, Error> {

		let head = self.store.head().map_err(&Error::StoreErr)?;
		let ctx = self.ctx_from_head(head, opts);

		let res = pipe::process_block(b, ctx);

		if let Ok(Some(ref tip)) = res {
			let chain_head = self.head.clone();
			let mut head = chain_head.lock().unwrap();
			*head = tip.clone();
		}

		res
	}

	/// Attempt to add a new header to the header chain. Only necessary during
	/// sync.
	pub fn process_block_header(&self,
	                            bh: &BlockHeader,
	                            opts: Options)
	                            -> Result<Option<Tip>, Error> {

		let head = self.store.get_header_head().map_err(&Error::StoreErr)?;
		let ctx = self.ctx_from_head(head, opts);

		pipe::process_block_header(bh, ctx)
	}

	fn ctx_from_head(&self, head: Tip, opts: Options) -> pipe::BlockContext {
		let mut opts_in = opts;
		if self.test_mode {
			opts_in = opts_in | EASY_POW;
		}
		pipe::BlockContext {
			opts: opts_in,
			store: self.store.clone(),
			adapter: self.adapter.clone(),
			head: head,
			lock: self.block_process_lock.clone(),
		}
	}

	/// Gets an unspent output from its commitment. With return None if the
	/// output
	/// doesn't exist or has been spent. This querying is done in a way that's
	/// constistent with the current chain state and more specifically the
	/// current
	/// branch it is on in case of forks.
	pub fn get_unspent(&self, output_ref: &Commitment) -> Option<Output> {
		// TODO use an actual UTXO tree
		// in the meantime doing it the *very* expensive way:
		//   1. check the output exists
		//   2. run the chain back from the head to check it hasn't been spent
		if let Ok(out) = self.store.get_output_by_commit(output_ref) {
			let head = none_err!(self.store.head());
			let mut block_h = head.last_block_h;
			loop {
				let b = none_err!(self.store.get_block(&block_h));
				for input in b.inputs {
					if input.commitment() == *output_ref {
						return None;
					}
				}
				if b.header.height == 1 {
					return Some(out);
				} else {
					block_h = b.header.previous;
				}
			}
		}
		None
	}

	/// Total difficulty at the head of the chain
	pub fn total_difficulty(&self) -> Difficulty {
		self.head.lock().unwrap().clone().total_difficulty
	}

	/// Get the tip that's also the head of the chain
	pub fn head(&self) -> Result<Tip, Error> {
		Ok(self.head.lock().unwrap().clone())
	}

	/// Block header for the chain head
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.store.head_header().map_err(&Error::StoreErr)
	}

	/// Gets a block header by hash
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		self.store.get_block(h).map_err(&Error::StoreErr)
	}

	/// Gets a block header by hash
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		self.store.get_block_header(h).map_err(&Error::StoreErr)
	}

	/// Gets the block header at the provided height
	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		self.store.get_header_by_height(height).map_err(&Error::StoreErr)
	}

	/// Get the tip of the header chain
	pub fn get_header_head(&self) -> Result<Tip, Error> {
		self.store.get_header_head().map_err(&Error::StoreErr)
	}

	/// Builds an iterator on blocks starting from the current chain head and
	/// running backward. Specialized to return information pertaining to block
	/// difficulty calculation (timestamp and previous difficulties).
	pub fn difficulty_iter(&self) -> store::DifficultyIter {
		let head = self.head.lock().unwrap();
		store::DifficultyIter::from(head.last_block_h, self.store.clone())
	}
}
