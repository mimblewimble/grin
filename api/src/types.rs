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

use std::sync::Arc;
use core::{core, global};
use chain;
use secp::pedersen;
use util;

/// The state of the current fork tip
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Tip {
	/// Height of the tip (max height of the fork)
	pub height: u64,
	// Last block pushed to the fork
	pub last_block_pushed: String,
	// Block previous to last
	pub prev_block_to_last: String,
	// Total difficulty accumulated on that fork
	pub total_difficulty: u64,
}

impl Tip {
	pub fn from_tip(tip: chain::Tip) -> Tip {
		Tip {
			height: tip.height,
			last_block_pushed: util::to_hex(tip.last_block_h.to_vec()),
			prev_block_to_last: util::to_hex(tip.prev_block_h.to_vec()),
			total_difficulty: tip.total_difficulty.into_num(),
		}
	}
}

/// Sumtrees 
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SumTrees {
	/// UTXO Root Hash
	pub utxo_root_hash: String,
	// UTXO Root Sum
	pub utxo_root_sum: String,
	// Rangeproof root hash
	pub range_proof_root_hash: String,
	// Kernel set root hash
	pub kernel_root_hash: String,

}

impl SumTrees {
	pub fn from_head(head: Arc<chain::Chain>) -> SumTrees {
		let roots=head.get_sumtree_roots();
		SumTrees {
			utxo_root_hash: util::to_hex(roots.0.hash.to_vec()),
			utxo_root_sum: util::to_hex(roots.0.sum.commit.0.to_vec()),
			range_proof_root_hash: util::to_hex(roots.1.hash.to_vec()),
			kernel_root_hash: util::to_hex(roots.2.hash.to_vec()),
		}
	}
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum OutputType {
	Coinbase,
	Transaction,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Output {
	/// The type of output Coinbase|Transaction
	pub output_type: OutputType,
	/// The homomorphic commitment representing the output's amount
	pub commit: pedersen::Commitment,
	/// A proof that the commitment is in the right range
	pub proof: pedersen::RangeProof,
	/// The height of the block creating this output
	pub height: u64,
	/// The lock height (earliest block this output can be spent)
	pub lock_height: u64,
}

impl Output {
	pub fn from_output(output: &core::Output, block_header: &core::BlockHeader) -> Output {
		let (output_type, lock_height) = match output.features {
			x if x.contains(core::transaction::COINBASE_OUTPUT) => {
				(OutputType::Coinbase, block_header.height + global::coinbase_maturity())
			}
			_ => (OutputType::Transaction, 0),
		};

		Output {
			output_type: output_type,
			commit: output.commit,
			proof: output.proof,
			height: block_header.height,
			lock_height: lock_height,
		}
	}
}

#[derive(Serialize, Deserialize)]
pub struct PoolInfo {
	/// Size of the pool
	pub pool_size: usize,
	/// Size of orphans
	pub orphans_size: usize,
	/// Total size of pool + orphans
	pub total_size: usize,
}
