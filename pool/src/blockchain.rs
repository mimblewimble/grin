// Copyright 2018 The Grin Developers
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

//! This file is (hopefully) temporary.
//!
//! It contains a trait based on (but not exactly equal to) the trait defined
//! for the blockchain Output set, discussed at
//! https://github.com/ignopeverell/grin/issues/29, and a dummy implementation
//! of said trait.
//! Notably, OutputDiff has been left off, and the question of how to handle
//! abstract return types has been deferred.

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
	/// Empty output set
	pub fn empty() -> DummyOutputSet {
		DummyOutputSet {
			outputs: HashMap::new(),
		}
	}

	/// roots
	pub fn root(&self) -> hash::Hash {
		hash::ZERO_HASH
	}

	/// apply a block
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

	/// create with block
	pub fn with_block(&mut self, b: &block::Block) {
		for input in &b.inputs {
			self.outputs.remove(&input.commitment());
		}
		for output in &b.outputs {
			self.outputs.insert(output.commitment(), output.clone());
		}
	}

	/// rewind
	pub fn rewind(&self, _: &block::Block) -> DummyOutputSet {
		DummyOutputSet {
			outputs: HashMap::new(),
		}
	}

	/// get an output
	pub fn get_output(&self, output_ref: &Commitment) -> Option<&transaction::Output> {
		self.outputs.get(output_ref)
	}

	fn clone(&self) -> DummyOutputSet {
		DummyOutputSet {
			outputs: self.outputs.clone(),
		}
	}

	/// only for testing: add an output to the map
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
	/// new dummy chain
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
	fn head_header(&self) -> Result<block::BlockHeader, PoolError> {
		let headers = self.block_headers.read().unwrap();
		if headers.len() > 0 {
			Ok(headers[0].clone())
		} else {
			Err(PoolError::GenericPoolError)
		}
	}

	// fn validate_raw_tx(&self, tx: &transaction::Transaction) -> Result<(),
	// PoolError> { 	tx.validate().map_err(|e| PoolError::InvalidTx(e))?;
	//
	// 	// TODO - rewrite this if statement
	// 	for x in &tx.inputs {
	// 		if self.output
	// 			.read()
	// 			.unwrap()
	// 			.get_output(&x.commitment())
	// 			.is_none()
	// 		{
	// 			return Err(PoolError::OutputNotFound);
	// 		}
	// 	}
	//
	// 	Ok(())
	// }

	fn validate_raw_txs(
		&self,
		txs: Vec<transaction::Transaction>,
		pre_tx: Option<&transaction::Transaction>,
	) -> Result<Vec<transaction::Transaction>, PoolError> {
		panic!("not yet implemented");
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

/// Dummy chain trait
pub trait DummyChain: BlockChain {
	/// update output set
	fn update_output_set(&mut self, new_output: DummyOutputSet);
	/// apply a block
	fn apply_block(&self, b: &block::Block);
	/// store header
	fn store_head_header(&self, block_header: &block::BlockHeader);
}
