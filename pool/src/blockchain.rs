// This file is (hopefully) temporary.
//
// It contains a trait based on (but not exactly equal to) the trait defined
// for the blockchain Output set, discussed at
// https://github.com/ignopeverell/grin/issues/29, and a dummy implementation
// of said trait.
// Notably, OutputDiff has been left off, and the question of how to handle
// abstract return types has been deferred.

use std::collections::HashMap;
use std::clone::Clone;
use std::sync::RwLock;

use core::core::{block, hash, transaction};
use core::core::{Input, OutputFeatures, OutputIdentifier};
use core::global;
use core::core::hash::Hashed;
use types::{BlockChain, PoolError};
use util::secp::pedersen::Commitment;

/// A DummyOutputSet for mocking up the chain
pub struct DummyOutputSet {
	outputs: HashMap<Commitment, transaction::Output>,
}

#[allow(dead_code)]
impl DummyOutputSet {
	pub fn empty() -> DummyOutputSet {
		DummyOutputSet {
			outputs: HashMap::new(),
		}
	}

	pub fn root(&self) -> hash::Hash {
		hash::ZERO_HASH
	}

	pub fn apply(&self, b: &block::Block) -> DummyOutputSet {
		let mut new_outputs = self.outputs.clone();

		for input in &b.inputs {
			new_outputs.remove(&input.commitment());
		}
		for output in &b.outputs {
			new_outputs.insert(output.commitment(), output.clone());
		}
		DummyOutputSet {
			outputs: new_outputs,
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

	pub fn rewind(&self, _: &block::Block) -> DummyOutputSet {
		DummyOutputSet {
			outputs: HashMap::new(),
		}
	}

	pub fn get_output(&self, output_ref: &Commitment) -> Option<&transaction::Output> {
		self.outputs.get(output_ref)
	}

	fn clone(&self) -> DummyOutputSet {
		DummyOutputSet {
			outputs: self.outputs.clone(),
		}
	}

	// only for testing: add an output to the map
	pub fn with_output(&self, output: transaction::Output) -> DummyOutputSet {
		let mut new_outputs = self.outputs.clone();
		new_outputs.insert(output.commitment(), output);
		DummyOutputSet {
			outputs: new_outputs,
		}
	}
}

/// A DummyChain is the mocked chain for playing with what methods we would
/// need
#[allow(dead_code)]
pub struct DummyChainImpl {
	output: RwLock<DummyOutputSet>,
	block_headers: RwLock<Vec<block::BlockHeader>>,
}

#[allow(dead_code)]
impl DummyChainImpl {
	pub fn new() -> DummyChainImpl {
		DummyChainImpl {
			output: RwLock::new(DummyOutputSet {
				outputs: HashMap::new(),
			}),
			block_headers: RwLock::new(vec![]),
		}
	}
}

impl BlockChain for DummyChainImpl {
	fn is_unspent(&self, output_ref: &OutputIdentifier) -> Result<hash::Hash, PoolError> {
		match self.output.read().unwrap().get_output(&output_ref.commit) {
			Some(_) => Ok(hash::Hash::zero()),
			None => Err(PoolError::GenericPoolError),
		}
	}

	fn is_matured(&self, input: &Input, height: u64) -> Result<(), PoolError> {
		if !input.features.contains(OutputFeatures::COINBASE_OUTPUT) {
			return Ok(());
		}
		let block_hash = input.block_hash.expect("requires a block hash");
		let headers = self.block_headers.read().unwrap();
		if let Some(h) = headers.iter().find(|x| x.hash() == block_hash) {
			if h.height + global::coinbase_maturity() < height {
				return Ok(());
			}
		}
		Err(PoolError::InvalidTx(transaction::Error::ImmatureCoinbase))
	}

	fn head_header(&self) -> Result<block::BlockHeader, PoolError> {
		let headers = self.block_headers.read().unwrap();
		if headers.len() > 0 {
			Ok(headers[0].clone())
		} else {
			Err(PoolError::GenericPoolError)
		}
	}
}

impl DummyChain for DummyChainImpl {
	fn update_output_set(&mut self, new_output: DummyOutputSet) {
		self.output = RwLock::new(new_output);
	}

	fn apply_block(&self, b: &block::Block) {
		self.output.write().unwrap().with_block(b);
		self.store_head_header(&b.header)
	}

	fn store_head_header(&self, block_header: &block::BlockHeader) {
		let mut headers = self.block_headers.write().unwrap();
		headers.insert(0, block_header.clone());
	}
}

pub trait DummyChain: BlockChain {
	fn update_output_set(&mut self, new_output: DummyOutputSet);
	fn apply_block(&self, b: &block::Block);
	fn store_head_header(&self, block_header: &block::BlockHeader);
}
