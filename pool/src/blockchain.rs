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

use secp::pedersen::Commitment;

use std::sync::RwLock;

use types::BlockChain;

/// A DummyUtxoSet for mocking up the chain 
pub struct DummyUtxoSet {
    outputs : HashMap<Commitment, transaction::Output>
}

impl DummyUtxoSet {
    pub fn empty() -> DummyUtxoSet{
        DummyUtxoSet{outputs: HashMap::new()}
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
        DummyUtxoSet{outputs: new_hashmap}
    }
    pub fn with_block(&mut self, b: &block::Block) {
        for input in &b.inputs {
            self.outputs.remove(&input.commitment());
        }
        for output in &b.outputs {
            self.outputs.insert(output.commitment(), output.clone());
        }
    }
    pub fn rewind(&self, b: &block::Block) -> DummyUtxoSet {
        DummyUtxoSet{outputs: HashMap::new()}
    }
    pub fn get_output(&self, output_ref: &Commitment) -> Option<&transaction::Output> {
        self.outputs.get(output_ref)
    }

    fn clone(&self) -> DummyUtxoSet {
        DummyUtxoSet{outputs: self.outputs.clone()}
    }

    // only for testing: add an output to the map
    pub fn add_output(&mut self, output: transaction::Output) {
        self.outputs.insert(output.commitment(), output);
    }
    // like above, but doesn't modify in-place so no mut ref needed
    pub fn with_output(&self, output: transaction::Output) -> DummyUtxoSet {
        let mut new_map = self.outputs.clone();
        new_map.insert(output.commitment(), output);
        DummyUtxoSet{outputs: new_map}
    }
}

/// A DummyChain is the mocked chain for playing with what methods we would
/// need
pub struct DummyChainImpl {
    utxo: RwLock<DummyUtxoSet>
}

impl DummyChainImpl {
    pub fn new() -> DummyChainImpl {
        DummyChainImpl{
            utxo: RwLock::new(DummyUtxoSet{outputs: HashMap::new()})}
    }
}

impl BlockChain for DummyChainImpl {
    fn get_unspent(&self, commitment: &Commitment) -> Option<transaction::Output> {
        self.utxo.read().unwrap().get_output(commitment).cloned()
    }
}

impl DummyChain for DummyChainImpl {
    fn update_utxo_set(&mut self, new_utxo: DummyUtxoSet) {
        self.utxo = RwLock::new(new_utxo);
    }
    fn apply_block(&self, b: &block::Block) {
        self.utxo.write().unwrap().with_block(b);
    }
}

pub trait DummyChain: BlockChain {
    fn update_utxo_set(&mut self, new_utxo: DummyUtxoSet);
    fn apply_block(&self, b: &block::Block);
}
