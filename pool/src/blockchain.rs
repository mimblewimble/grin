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

/// A DummyUtxoSet for mocking up the chain 
pub struct DummyUtxoSet {
    outputs : HashMap<hash::Hash, transaction::Output>
}

impl DummyUtxoSet {
    pub fn root(&self) -> hash::Hash {
        hash::ZERO_HASH
    }
    pub fn apply(&self, b: block::Block) -> DummyUtxoSet {
        DummyUtxoSet{}
    }
    pub fn rewind(&self, b: block::Block) -> DummyUtxoSet {
        DummyUtxoSet{}
    }
    pub fn get_output(&self, output_ref: &hash::Hash) -> Option<&transaction::Output> {
        self.outputs.get(output_ref)
    }

    // only for testing: add an output to the map
    pub fn add_output(&mut self, output: transaction::Output) {
        self.outputs.insert(&output.hash(), output);
    }
    // like above, but doesn't modify in-place so no mut ref needed
    pub fn with_output(&self, output: transaction::Output) -> DummyUtxoSet {
        DummyUtxoSet{outputs: self.outputs.clone().insert(&output.hash(),
            output)}
    }
}

/// A DummyChain is the mocked chain for playing with what methods we would
/// need
pub struct DummyChain {
    utxo: DummyUtxoSet
}

impl DummyChain {
    pub fn get_best_utxo_set(&self) -> &DummyUtxoSet {
        &self.utxo
    }
    pub fn update_utxo_set(&mut self, new_utxo: DummyUtxoSet) {
        self.utxo = new_utxo;
    }
}
