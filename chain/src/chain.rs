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

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use secp::pedersen::Commitment;

use core::core::{Block, BlockHeader, Output};
use core::core::target::Difficulty;
use core::core::hash::Hash;
use grin_store::Error::NotFoundErr;
use pipe;
use store;
use types::*;

use core::global::{MiningParameterMode,MINING_PARAMETER_MODE};

const MAX_ORPHANS: usize = 20;

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
	orphans: Arc<Mutex<VecDeque<(Options, Block)>>>,

	//POW verification function
	pow_verifier: fn(&BlockHeader, u32) -> bool,
}

unsafe impl Sync for Chain {}
unsafe impl Send for Chain {}

impl Chain {

	/// Check whether the chain exists. If not, the call to 'init' will
	/// expect an already mined genesis block. This keeps the chain free
	/// from needing to know about the mining implementation
	pub fn chain_exists(db_root: String)->bool {
		let chain_store = store::ChainKVStore::new(db_root).unwrap();
		match chain_store.head() {
			Ok(_) => {true},
			Err(NotFoundErr) => false,
			Err(_) => false,
		}
	}

	/// Initializes the blockchain and returns a new Chain instance. Does a
	/// check
	/// on the current chain head to make sure it exists and creates one based
	/// on
	/// the genesis block if necessary.
	pub fn init(
		db_root: String,
		adapter: Arc<ChainAdapter>,
		gen_block: Option<Block>,
		pow_verifier: fn(&BlockHeader, u32) -> bool,
	) -> Result<Chain, Error> {
		let chain_store = store::ChainKVStore::new(db_root)?;

		// check if we have a head in store, otherwise the genesis block is it
		let head = match chain_store.head() {
			Ok(tip) => tip,
			Err(NotFoundErr) => {
				if let None = gen_block {
					return Err(Error::GenesisBlockRequired);
				}

				let gen = gen_block.unwrap();
				chain_store.save_block(&gen)?;

				// saving a new tip based on genesis
				let tip = Tip::new(gen.hash());
				chain_store.save_head(&tip)?;
				info!("Saved genesis block with hash {}", gen.hash());
				tip
			}
			Err(e) => return Err(Error::StoreErr(e)),
		};

        // TODO - confirm this was safe to remove based on code above?
		// let head = chain_store.head()?;


		Ok(Chain {
			store: Arc::new(chain_store),
			adapter: adapter,
			head: Arc::new(Mutex::new(head)),
			block_process_lock: Arc::new(Mutex::new(true)),
			orphans: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_ORPHANS + 1))),
			pow_verifier: pow_verifier,
		})
	}

	/// Attempt to add a new block to the chain. Returns the new chain tip if it
	/// has been added to the longest chain, None if it's added to an (as of
	/// now) orphan chain.
	pub fn process_block(&self, b: Block, opts: Options) -> Result<Option<Tip>, Error> {

		let head = self.store.head().map_err(&Error::StoreErr)?;
		let ctx = self.ctx_from_head(head, opts);

		let res = pipe::process_block(&b, ctx);

		match res {
			Ok(Some(ref tip)) => {
				// block got accepted and extended the head, updating our head
				let chain_head = self.head.clone();
				{
					let mut head = chain_head.lock().unwrap();
					*head = tip.clone();
				}

				self.check_orphans();
			}
			Err(Error::Orphan) => {
				let mut orphans = self.orphans.lock().unwrap();
				orphans.push_front((opts, b));
				orphans.truncate(MAX_ORPHANS);
			}
			_ => {}
		}

		res
	}

	/// Attempt to add a new header to the header chain. Only necessary during
	/// sync.
	pub fn process_block_header(
		&self,
		bh: &BlockHeader,
		opts: Options,
	) -> Result<Option<Tip>, Error> {

		let head = self.store.get_header_head().map_err(&Error::StoreErr)?;
		let ctx = self.ctx_from_head(head, opts);

		pipe::process_block_header(bh, ctx)
	}

	fn ctx_from_head(&self, head: Tip, opts: Options) -> pipe::BlockContext {
		let opts_in = opts;
		let param_ref=MINING_PARAMETER_MODE.read().unwrap();
		let opts_in = match *param_ref {
			MiningParameterMode::AutomatedTesting => opts_in | EASY_POW,
			MiningParameterMode::UserTesting => opts_in | EASY_POW,
			MiningParameterMode::Production => opts_in,
		};

		pipe::BlockContext {
			opts: opts_in,
			store: self.store.clone(),
			adapter: self.adapter.clone(),
			head: head,
			pow_verifier: self.pow_verifier,
			lock: self.block_process_lock.clone(),
		}
	}

    /// Pop orphans out of the queue and check if we can now accept them.
	fn check_orphans(&self) {
		// first check how many we have to retry, unfort. we can't extend the lock
		// in the loop as it needs to be freed before going in process_block
		let orphan_count;
		{
			let orphans = self.orphans.lock().unwrap();
			orphan_count = orphans.len();
		}

		// pop each orphan and retry, if still orphaned, will be pushed again
		for _ in 0..orphan_count {
			let popped;
			{
				let mut orphans = self.orphans.lock().unwrap();
				popped = orphans.pop_back();
			}
			if let Some((opts, orphan)) = popped {
				let _process_result = self.process_block(orphan, opts);
			}
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
		self.store.get_header_by_height(height).map_err(
			&Error::StoreErr,
		)
	}

    /// Gets the block header by the provided output commitment
    pub fn get_block_header_by_output_commit(&self, commit: &Commitment) -> Result<BlockHeader, Error> {
        self.store.get_block_header_by_output_commit(commit).map_err(
            &Error::StoreErr,
        )
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
