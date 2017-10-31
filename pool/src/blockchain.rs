// This file is (hopefully) temporary.
//
// It contains a trait based on (but not exactly equal to) the trait defined
// for the blockchain UTXO set, discussed at
// https://github.com/ignopeverell/grin/issues/29, and a dummy implementation
// of said trait.
// Notably, UtxoDiff has been left off, and the question of how to handle
// abstract return types has been deferred.

use core::core::hash;
use core::core::block;
use core::core::transaction;

use std::collections::HashMap;
use std::clone::Clone;

use util::secp::pedersen::Commitment;

use std::sync::RwLock;

use types::{BlockChain, PoolError};

#[derive(Debug)]
pub struct DummyBlockHeaderIndex {
	block_headers: HashMap<Commitment, block::BlockHeader>,
}

impl DummyBlockHeaderIndex {
	pub fn insert(&mut self, commit: Commitment, block_header: block::BlockHeader) {
		self.block_headers.insert(commit, block_header);
	}

	pub fn get_block_header_by_output_commit(
		&self,
		commit: Commitment,
	) -> Result<&block::BlockHeader, PoolError> {
		match self.block_headers.get(&commit) {
			Some(h) => Ok(h),
			None => Err(PoolError::GenericPoolError),
		}
	}
}

/// A DummyUtxoSet for mocking up the chain
pub struct DummyUtxoSet {
	outputs: HashMap<Commitment, transaction::Output>,
}

#[allow(dead_code)]
impl DummyUtxoSet {
	pub fn empty() -> DummyUtxoSet {
		DummyUtxoSet {
			outputs: HashMap::new(),
		}
	}
	pub fn root(&self) -> hash::Hash {
		hash::ZERO_HASH
	}
	pub fn apply(&self, b: &block::Block) -> DummyUtxoSet {
		let mut new_hashmap = self.outputs.clone();
		for input in &b.inputs {
			new_hashmap.remove(&input.commitment());
		}
		for output in &b.outputs {
			new_hashmap.insert(output.commitment(), output.clone());
		}
		DummyUtxoSet {
			outputs: new_hashmap,
		}
	}
	pub fn with_block(&mut self, b: &block::Block) {
		for input in &b.inputs {
			self.outputs.remove(&input.commitment());
		}
		for output in &b.outputs {
			self.outputs.insert(output.commitment(), output.clone());
		}
	}
	pub fn rewind(&self, _: &block::Block) -> DummyUtxoSet {
		DummyUtxoSet {
			outputs: HashMap::new(),
		}
	}
	pub fn get_output(&self, output_ref: &Commitment) -> Option<&transaction::Output> {
		self.outputs.get(output_ref)
	}

	fn clone(&self) -> DummyUtxoSet {
		DummyUtxoSet {
			outputs: self.outputs.clone(),
		}
	}

	// only for testing: add an output to the map
	pub fn add_output(&mut self, output: transaction::Output) {
		self.outputs.insert(output.commitment(), output);
	}
	// like above, but doesn't modify in-place so no mut ref needed
	pub fn with_output(&self, output: transaction::Output) -> DummyUtxoSet {
		let mut new_map = self.outputs.clone();
		new_map.insert(output.commitment(), output);
		DummyUtxoSet { outputs: new_map }
	}
}

/// A DummyChain is the mocked chain for playing with what methods we would
/// need
#[allow(dead_code)]
pub struct DummyChainImpl {
	utxo: RwLock<DummyUtxoSet>,
	block_headers: RwLock<DummyBlockHeaderIndex>,
	head_header: RwLock<Vec<block::BlockHeader>>,
}

#[allow(dead_code)]
impl DummyChainImpl {
	pub fn new() -> DummyChainImpl {
		DummyChainImpl {
			utxo: RwLock::new(DummyUtxoSet {
				outputs: HashMap::new(),
			}),
			block_headers: RwLock::new(DummyBlockHeaderIndex {
				block_headers: HashMap::new(),
			}),
			head_header: RwLock::new(vec![]),
		}
	}
}

impl BlockChain for DummyChainImpl {
	fn get_unspent(&self, commitment: &Commitment) -> Result<transaction::Output, PoolError> {
		let output = self.utxo.read().unwrap().get_output(commitment).cloned();
		match output {
			Some(o) => Ok(o),
			None => Err(PoolError::GenericPoolError),
		}
	}

	fn get_block_header_by_output_commit(
		&self,
		commit: &Commitment,
	) -> Result<block::BlockHeader, PoolError> {
		match self.block_headers
			.read()
			.unwrap()
			.get_block_header_by_output_commit(*commit)
		{
			Ok(h) => Ok(h.clone()),
			Err(e) => Err(e),
		}
	}

	fn head_header(&self) -> Result<block::BlockHeader, PoolError> {
		let headers = self.head_header.read().unwrap();
		if headers.len() > 0 {
			Ok(headers[0].clone())
		} else {
			Err(PoolError::GenericPoolError)
		}
	}
}

impl DummyChain for DummyChainImpl {
	fn update_utxo_set(&mut self, new_utxo: DummyUtxoSet) {
		self.utxo = RwLock::new(new_utxo);
	}
	fn apply_block(&self, b: &block::Block) {
		self.utxo.write().unwrap().with_block(b);
	}
	fn store_header_by_output_commitment(
		&self,
		commitment: Commitment,
		block_header: &block::BlockHeader,
	) {
		self.block_headers
			.write()
			.unwrap()
			.insert(commitment, block_header.clone());
	}
	fn store_head_header(&self, block_header: &block::BlockHeader) {
		let mut h = self.head_header.write().unwrap();
		h.clear();
		h.insert(0, block_header.clone());
	}
}

pub trait DummyChain: BlockChain {
	fn update_utxo_set(&mut self, new_utxo: DummyUtxoSet);
	fn apply_block(&self, b: &block::Block);
	fn store_header_by_output_commitment(
		&self,
		commitment: Commitment,
		block_header: &block::BlockHeader,
	);
	fn store_head_header(&self, block_header: &block::BlockHeader);
}
