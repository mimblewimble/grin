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
use std::sync::{Arc, Mutex, RwLock};

use util::secp::pedersen::{Commitment, RangeProof};

use core::core::SumCommit;
use core::core::pmmr::{HashSum, NoSum};

use core::core::{Block, BlockHeader, Output, TxKernel};
use core::core::target::Difficulty;
use core::core::hash::Hash;
use grin_store::Error::NotFoundErr;
use pipe;
use store;
use sumtree;
use types::*;
use util::LOGGER;

const MAX_ORPHANS: usize = 50;

/// Facade to the blockchain block processing pipeline and storage. Provides
/// the current view of the UTXO set according to the chain state. Also
/// maintains locking for the pipeline to avoid conflicting processing.
pub struct Chain {
	store: Arc<ChainStore>,
	adapter: Arc<ChainAdapter>,

	head: Arc<Mutex<Tip>>,
	orphans: Arc<Mutex<VecDeque<(Options, Block)>>>,
	sumtrees: Arc<RwLock<sumtree::SumTrees>>,

	// POW verification function
	pow_verifier: fn(&BlockHeader, u32) -> bool,
}

unsafe impl Sync for Chain {}
unsafe impl Send for Chain {}

impl Chain {
	/// Check whether the chain exists. If not, the call to 'init' will
	/// expect an already mined genesis block. This keeps the chain free
	/// from needing to know about the mining implementation
	pub fn chain_exists(db_root: String) -> bool {
		let chain_store = store::ChainKVStore::new(db_root).unwrap();
		match chain_store.head() {
			Ok(_) => true,
			Err(NotFoundErr) => false,
			Err(_) => false,
		}
	}

	/// Initializes the blockchain and returns a new Chain instance. Does a check
	/// on the current chain head to make sure it exists and creates one based
	/// on the genesis block if necessary.
	pub fn init(
		db_root: String,
		adapter: Arc<ChainAdapter>,
		gen_block: Option<Block>,
		pow_verifier: fn(&BlockHeader, u32) -> bool,
	) -> Result<Chain, Error> {
		let chain_store = store::ChainKVStore::new(db_root.clone())?;

		// check if we have a head in store, otherwise the genesis block is it
		let head = match chain_store.head() {
			Ok(tip) => tip,
			Err(NotFoundErr) => {
				if let None = gen_block {
					return Err(Error::GenesisBlockRequired);
				}

				let gen = gen_block.unwrap();
				let tip = Tip::new(gen.hash());
				chain_store.save_block(&gen)?;
				chain_store.setup_height(&gen.header, &tip)?;

				// saving a new tip based on genesis
				chain_store.save_head(&tip)?;
				info!(
					LOGGER,
					"Saved genesis block with hash: {:?}, nonce: {:?}, pow: {:?}",
					gen.hash(),
					gen.header.nonce,
					gen.header.pow,
				);
				tip
			}
			Err(e) => return Err(Error::StoreErr(e, "chain init load head".to_owned())),
		};

		// make sure sync_head is available for later use
		let _ = match chain_store.get_sync_head() {
			Ok(tip) => tip,
			Err(NotFoundErr) => {
				let tip = chain_store.head().unwrap();
				chain_store.save_sync_head(&tip)?;
				tip
			},
			Err(e) => return Err(Error::StoreErr(e, "chain init sync head".to_owned())),
		};

		info!(
			LOGGER,
			"Chain init: {:?}",
			head,
		);

		let store = Arc::new(chain_store);
		let sumtrees = sumtree::SumTrees::open(db_root, store.clone())?;

		Ok(Chain {
			store: store,
			adapter: adapter,
			head: Arc::new(Mutex::new(head)),
			orphans: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_ORPHANS + 1))),
			sumtrees: Arc::new(RwLock::new(sumtrees)),
			pow_verifier: pow_verifier,
		})
	}

	/// Attempt to add a new block to the chain. Returns the new chain tip if it
	/// has been added to the longest chain, None if it's added to an (as of
	/// now) orphan chain.
	pub fn process_block(&self, b: Block, opts: Options)
		-> Result<(Option<Tip>, Vec<Hash>), Error> {

		let head = self.store
			.head()
			.map_err(|e| Error::StoreErr(e, "chain load head".to_owned()))?;
		let height = head.height;
		let ctx = self.ctx_from_head(head, opts);

		let res = pipe::process_block(&b, ctx);

		let mut accepted_orphans = vec![];
		match res {
			Ok(Some(ref tip)) => {
				// block got accepted and extended the head, updating our head
				let chain_head = self.head.clone();
				{
					let mut head = chain_head.lock().unwrap();
					*head = tip.clone();
				}

				// notifying other parts of the system of the update
				if !opts.intersects(SYNC) {
					// broadcast the block
					let adapter = self.adapter.clone();
					adapter.block_accepted(&b);
				}
				accepted_orphans = self.check_orphans();
			},
			Ok(None) => {
				// block got accepted but we did not extend the head
				// so its on a fork (or is the start of a new fork)
				// broadcast the block out so everyone knows about the fork
				//
				// TODO - This opens us to an amplification attack on blocks
				// mined at a low difficulty. We should suppress really old blocks
				// or less relevant blocks somehow.
				// We should also probably consider banning nodes that send us really old blocks.
				//
				if !opts.intersects(SYNC) {
					// broadcast the block
					let adapter = self.adapter.clone();
					adapter.block_accepted(&b);
				}
				// We accepted a block here so there is a chance we can now accept
				// one or more orphans.
				accepted_orphans = self.check_orphans();
			},
			Err(Error::Orphan) => {
				// TODO - Do we want to check that orphan height is > current height?
				// TODO - Just check heights here? Or should we be checking total_difficulty as well?
				let block_hash = b.hash();
				if b.header.height < height + (MAX_ORPHANS as u64) {
					let mut orphans = self.orphans.lock().unwrap();
					orphans.push_front((opts, b));
					orphans.truncate(MAX_ORPHANS);
					debug!(
						LOGGER,
						"process_block: orphan: {:?}, # orphans {}",
						block_hash,
						orphans.len(),
					);
				} else {
					debug!(
						LOGGER,
						"process_block: orphan: {:?}, (dropping, height {} vs {})",
						block_hash,
						b.header.height,
						height,
					);
				}
			},
			Err(Error::Unfit(ref msg)) => {
				debug!(
					LOGGER,
					"Block {} at {} is unfit at this time: {}",
					b.hash(),
					b.header.height,
					msg
				);
			}
			Err(ref e) => {
				info!(
					LOGGER,
					"Rejected block {} at {}: {:?}",
					b.hash(),
					b.header.height,
					e
				);
			}
		}

		res.map(|tip| (tip, accepted_orphans))
	}

	/// Attempt to add a new header to the header chain.
	/// This is only ever used during sync and uses sync_head.
	pub fn sync_block_header(
		&self,
		bh: &BlockHeader,
		opts: Options,
	) -> Result<Option<Tip>, Error> {
		let sync_head = self.get_sync_head()?;
		let header_head = self.get_header_head()?;
		let sync_ctx = self.ctx_from_head(sync_head, opts);
		let header_ctx = self.ctx_from_head(header_head, opts);
		pipe::sync_block_header(bh, sync_ctx, header_ctx)
	}

	fn ctx_from_head(&self, head: Tip, opts: Options) -> pipe::BlockContext {
		pipe::BlockContext {
			opts: opts,
			store: self.store.clone(),
			head: head,
			pow_verifier: self.pow_verifier,
			sumtrees: self.sumtrees.clone(),
		}
	}

	/// Check if hash is for a known orphan.
	pub fn is_orphan(&self, hash: &Hash) -> bool {
		let orphans = self.orphans.lock().unwrap();
		orphans.iter().any(|&(_, ref x)| x.hash() == hash.clone())
	}

	/// Pop orphans out of the queue and check if we can now accept them.
	fn check_orphans(&self) -> Vec<Hash> {
		// first check how many we have to retry, unfort. we can't extend the lock
		// in the loop as it needs to be freed before going in process_block
		let orphan_count;
		{
			let orphans = self.orphans.lock().unwrap();
			orphan_count = orphans.len();
		}
		debug!(LOGGER, "check_orphans: # orphans {}", orphan_count);

		// pop each orphan and retry, if still orphaned, will be pushed again
		let mut processed = vec![];
		for _ in 0..orphan_count {
			let popped;
			{
				let mut orphans = self.orphans.lock().unwrap();
				popped = orphans.pop_back();
			}
			if let Some((opts, orphan)) = popped {
				let o_hash = orphan.hash();
				if let Ok(_) = self.process_block(orphan, opts) {
					processed.push(o_hash);
				}
			}
		}
		processed
	}

	/// Gets an unspent output from its commitment. With return None if the
	/// output doesn't exist or has been spent. This querying is done in a
	/// way that's consistent with the current chain state and more
	/// specifically the current winning fork.
	pub fn get_unspent(&self, output_ref: &Commitment) -> Result<Output, Error> {
		let sumtrees = self.sumtrees.read().unwrap();
		let is_unspent = sumtrees.is_unspent(output_ref)?;
		if is_unspent {
			self.store
				.get_output_by_commit(output_ref)
				.map_err(|e| Error::StoreErr(e, "chain get unspent".to_owned()))
		} else {
			Err(Error::OutputNotFound)
		}
	}

	/// Checks whether an output is unspent
	pub fn is_unspent(&self, output_ref: &Commitment) -> Result<bool, Error> {
		let sumtrees = self.sumtrees.read().unwrap();
		sumtrees.is_unspent(output_ref)
	}

	/// Sets the sumtree roots on a brand new block by applying the block on the
	/// current sumtree state.
	pub fn set_sumtree_roots(&self, b: &mut Block) -> Result<(), Error> {
		let mut sumtrees = self.sumtrees.write().unwrap();

		let roots = sumtree::extending(&mut sumtrees, |extension| {
			// apply the block on the sumtrees and check the resulting root
			extension.apply_block(b)?;
			extension.force_rollback();
			Ok(extension.roots())
		})?;

		b.header.utxo_root = roots.0.hash;
		b.header.range_proof_root = roots.1.hash;
		b.header.kernel_root = roots.2.hash;
		Ok(())
	}

	/// returs sumtree roots
	pub fn get_sumtree_roots(
		&self,
	) -> (
		HashSum<SumCommit>,
		HashSum<NoSum<RangeProof>>,
		HashSum<NoSum<TxKernel>>,
	) {
		let mut sumtrees = self.sumtrees.write().unwrap();
		sumtrees.roots()
	}

	/// Reset the header head to the same as the main head. When sync is running,
	/// the header head will go ahead to try to download as many as possible.
	/// However if a block, when fully received, is found invalid, the header
	/// head need to backtrack to the last known valid position.
	pub fn reset_header_head(&self) -> Result<(), Error> {
		let head = self.head.lock().unwrap();
		debug!(LOGGER, "Reset header head to {} at {}",
					head.last_block_h, head.height);
		self.store.save_header_head(&head).map_err(From::from)
	}

	/// returns the last n nodes inserted into the utxo sum tree
	/// returns sum tree hash plus output itself (as the sum is contained
	/// in the output anyhow)
	pub fn get_last_n_utxo(&self, distance: u64) -> Vec<(Hash, Output)> {
		let mut sumtrees = self.sumtrees.write().unwrap();
		let mut return_vec = Vec::new();
		let sum_nodes = sumtrees.last_n_utxo(distance);
		for sum_commit in sum_nodes {
			let output = self.store.get_output_by_commit(&sum_commit.sum.commit);
			return_vec.push((sum_commit.hash, output.unwrap()));
		}
		return_vec
	}

	/// as above, for rangeproofs
	pub fn get_last_n_rangeproof(&self, distance: u64) -> Vec<HashSum<NoSum<RangeProof>>> {
		let mut sumtrees = self.sumtrees.write().unwrap();
		sumtrees.last_n_rangeproof(distance)
	}

	/// as above, for kernels
	pub fn get_last_n_kernel(&self, distance: u64) -> Vec<HashSum<NoSum<TxKernel>>> {
		let mut sumtrees = self.sumtrees.write().unwrap();
		sumtrees.last_n_kernel(distance)
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
		self.store
			.head_header()
			.map_err(|e| Error::StoreErr(e, "chain head header".to_owned()))
	}

	/// Gets a block header by hash
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		self.store
			.get_block(h)
			.map_err(|e| Error::StoreErr(e, "chain get block".to_owned()))
	}

	/// Gets a block header by hash
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		self.store
			.get_block_header(h)
			.map_err(|e| Error::StoreErr(e, "chain get header".to_owned()))
	}

	/// Gets the block header at the provided height
	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		self.store.get_header_by_height(height).map_err(|e| {
			Error::StoreErr(e, "chain get header by height".to_owned())
		})
	}

	/// Verifies the given block header is actually on the current chain.
	/// Checks the header_by_height index to verify the header is where we say it is
	pub fn is_on_current_chain(&self, header: &BlockHeader) -> Result<(), Error> {
		self.store.is_on_current_chain(header).map_err(|e| {
			Error::StoreErr(e, "chain is_on_current_chain".to_owned())
		})
	}

	/// Gets the block header by the provided output commitment
	pub fn get_block_header_by_output_commit(
		&self,
		commit: &Commitment,
	) -> Result<BlockHeader, Error> {
		self.store
			.get_block_header_by_output_commit(commit)
			.map_err(|e| Error::StoreErr(e, "chain get commitment".to_owned()))
	}

	/// Get the tip of the current "sync" header chain.
	/// This may be significantly different to current header chain.
	pub fn get_sync_head(&self) -> Result<Tip, Error> {
		self.store
			.get_sync_head()
			.map_err(|e| Error::StoreErr(e, "chain get sync head".to_owned()))
	}

	/// Get the tip of the header chain.
	pub fn get_header_head(&self) -> Result<Tip, Error> {
		self.store
			.get_header_head()
			.map_err(|e| Error::StoreErr(e, "chain get header head".to_owned()))
	}

	/// Builds an iterator on blocks starting from the current chain head and
	/// running backward. Specialized to return information pertaining to block
	/// difficulty calculation (timestamp and previous difficulties).
	pub fn difficulty_iter(&self) -> store::DifficultyIter {
		let head = self.head.lock().unwrap();
		store::DifficultyIter::from(head.last_block_h, self.store.clone())
	}
}
