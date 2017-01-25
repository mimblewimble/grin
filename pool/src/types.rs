// Copyright 2017 The Grin Developers
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

//! Base types for the transaction pool implementation.

use std::vec::Vec;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::Weak;
use std::cell::RefCell;
use std::collections::HashMap;

use time;

use core::core;


/// An entry in the transaction pool.
/// Embeds the transaction itself, as well as a few pool-related accounting
/// fields.
/// Each entry has some information about its parent and its descendants. 
pub struct PoolEntry {
	pub tx_hash: core::hash::Hash,
	pub descendants: Vec<Arc<RefCell<PoolEntry>>>, 
    pub parents: Vec<Parent>,

	size_estimate: u64,
    pub receive_ts: time::Tm,
}

impl PoolEntry {
    fn is_orphaned(&self) -> bool {
        for i in self.parents {
            match i {
                Parent::Unknown | Parent::OrphanTransaction{..} => return true,
                _ => continue,
            }
        }
        false
    }
}

/// Rough first pass: the trait representing where we heard about a tx from.
pub trait TxSource {
    /// Human-readable name used for logging and errors.
    fn debug_name(&self) -> &str;
    /// Unique identifier used to distinguish this peer from others.
    fn identifier(&self) -> &str;
}

/// This enum describes the parent for a given input of a transaction.
#[derive(Clone)]
enum Parent {
    Unknown,
    BlockTransaction{hash: core::hash::Hash},
    PoolTransaction{hash: core::hash::Hash, tx_ref: Weak<RefCell<PoolEntry>>},
    OrphanTransaction{hash: core::hash::Hash, tx_ref: Weak<RefCell<PoolEntry>>},
}

enum PoolError {
    Invalid,
    Orphan,
}


/// The pool itself.
/// The transactions HashMap holds ownership of all transactions in the pool,
/// keyed by their transaction hash.
/// The primary data structure holding pool entries is the list of roots,
/// defined as pool entries with exclusively BlockTransactions as parents.
/// In this first pass, orphans are in the same output map as regular txs.
struct TransactionPool {
    pub transactions: HashMap<core::hash::Hash, Box<core::transaction::Transaction>>,

    roots : RwLock<Vec<Arc<RefCell<PoolEntry>>>>,
    orphan_roots : RwLock<Vec<Arc<RefCell<PoolEntry>>>>,
    by_output : RwLock<HashMap<core::hash::Hash, Weak<RefCell<PoolEntry>>>>,
}

impl TransactionPool {
    pub fn add_to_memory_pool(&self, source: TxSource, tx: core::transaction::Transaction) -> Result<(), PoolError> {
        // Placeholder: validation
        //tx.verify_sig;

        // Find the parent transactions
        // Using unwrap here: the only possible error is a poisonError, which
        // we don't have a good recovery for.
        // If this becomes an issue, we can rebuild the map from the graph
        // representations.
        let output_map = self.by_output.read().unwrap();
        let parents = vec![Parent::Unknown; tx.inputs.len()]; 
        for (i, input) in tx.inputs.iter().enumerate() {
            // First, check the confirmed UTXO state.
            
            // Next, check against pool state.
            let i_parent = match output_map.get(&input.output_hash()) {
                None => Parent::Unknown,
                Some(p) => parent_from_weak_ref(input.output_hash(), p),
            };
            parents[i] = i_parent;
        }
        Ok(())
    }
}

fn parent_from_weak_ref(h: core::hash::Hash, p: &Weak<RefCell<PoolEntry>>) -> Parent {
    p.upgrade().and_then(|x| parent_from_tx_ref(h, x)).unwrap_or(Parent::Unknown)
}

fn parent_from_tx_ref(h: core::hash::Hash, tx_ref: Arc<RefCell<PoolEntry>>) -> Parent {
    if tx_ref.borrow().is_orphaned() {
        return Parent::OrphanTransaction{hash: h, tx_ref: tx_ref.downgrade()};
    }
    Parent::PoolTransaction{hash: h, tx_ref: tx_ref}
}
