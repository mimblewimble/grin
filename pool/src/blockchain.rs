// This file is (hopefully) temporary.
//
// It contains a trait based on (but not exactly equal to) the trait defined
// for the blockchain UTXO set, discussed at
// https://github.com/ignopeverell/grin/issues/29, and a dummy implementation
// of said trait.
// Notably, UtxoDiff has been left off, and the question of how to handle
// abstract return types has been deferred.

use std::collections::HashMap;
use std::clone::Clone;
use std::sync::RwLock;

use core::core::hash;
use core::core::hash::Hashed;
use core::core::block;
use core::core::transaction;
use types::{BlockChain, PoolError};
use util::secp::pedersen::Commitment;


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
		let mut new_outputs = self.outputs.clone();

		for input in &b.inputs {
			new_outputs.remove(&input.commitment());
		}
		for output in &b.outputs {
			new_outputs.insert(output.commitment(), output.clone());
		}
		DummyUtxoSet {
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
	pub fn with_output(&self, output: transaction::Output) -> DummyUtxoSet {
		let mut new_outputs = self.outputs.clone();
		new_outputs.insert(output.commitment(), output);
		DummyUtxoSet {
			outputs: new_outputs,
		}
	}
}

/// A DummyChain is the mocked chain for playing with what methods we would
/// need
#[allow(dead_code)]
pub struct DummyChainImpl {
	utxo: RwLock<DummyUtxoSet>,
	head_header: RwLock<Vec<block::BlockHeader>>,
}

#[allow(dead_code)]
impl DummyChainImpl {
	pub fn new() -> DummyChainImpl {
		DummyChainImpl {
			utxo: RwLock::new(DummyUtxoSet {
				outputs: HashMap::new(),
			}),
			head_header: RwLock::new(vec![]),
		}
	}
}

impl BlockChain for DummyChainImpl {
	fn get_unspent(&self, commitment: &Commitment) -> Result<hash::Hash, PoolError> {
		match self.utxo.read().unwrap().get_output(commitment) {
			Some(x) => Ok(x.hash()),
			None => Err(PoolError::GenericPoolError),
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

	fn verify_coinbase_maturity(
		&self,
		input: &transaction::Input,
		height: u64,
	) -> Result<(), PoolError> {
		panic!("where are we calling this from???");
		
		// let out = OutputIdentifier::from_input(&input);
		// let block = self.get_block(&input.out_block)?;
		// block.verify_coinbase_maturity(&out, height)
		// 	.map_err(|x| Error::ImmatureCoinbase);

	}
}

impl DummyChain for DummyChainImpl {
	fn update_utxo_set(&mut self, new_utxo: DummyUtxoSet) {
		self.utxo = RwLock::new(new_utxo);
	}
	fn apply_block(&self, b: &block::Block) {
		self.utxo.write().unwrap().with_block(b);
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
	fn store_head_header(&self, block_header: &block::BlockHeader);
}
