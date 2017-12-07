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
use core::core::hash::Hashed;
use chain;
use util::secp::pedersen;
use rest::*;
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
		let roots = head.get_sumtree_roots();
		SumTrees {
			utxo_root_hash: util::to_hex(roots.0.hash.to_vec()),
			utxo_root_sum: util::to_hex(roots.0.sum.commit.0.to_vec()),
			range_proof_root_hash: util::to_hex(roots.1.hash.to_vec()),
			kernel_root_hash: util::to_hex(roots.2.hash.to_vec()),
		}
	}
}

/// Wrapper around a list of sumtree nodes, so it can be
/// presented properly via json
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SumTreeNode {
	// The hash
	pub hash: String,
	// Output (if included)
	pub output: Option<OutputPrintable>,
}

impl SumTreeNode {
	pub fn get_last_n_utxo(chain: Arc<chain::Chain>, distance: u64) -> Vec<SumTreeNode> {
		let mut return_vec = Vec::new();
		let last_n = chain.get_last_n_utxo(distance);
		for elem_output in last_n {
			let header = chain
				.get_block_header_by_output_commit(&elem_output.1.commit)
				.map_err(|_| Error::NotFound);
			// Need to call further method to check if output is spent
			let mut output = OutputPrintable::from_output(&elem_output.1, &header.unwrap(),true);
			if let Ok(_) = chain.get_unspent(&elem_output.1.commit) {
				output.spent = false;
			}
			return_vec.push(SumTreeNode {
				hash: util::to_hex(elem_output.0.to_vec()),
				output: Some(output),
			});
		}
		return_vec
	}

	pub fn get_last_n_rangeproof(head: Arc<chain::Chain>, distance: u64) -> Vec<SumTreeNode> {
		let mut return_vec = Vec::new();
		let last_n = head.get_last_n_rangeproof(distance);
		for elem in last_n {
			return_vec.push(SumTreeNode {
				hash: util::to_hex(elem.hash.to_vec()),
				output: None,
			});
		}
		return_vec
	}

	pub fn get_last_n_kernel(head: Arc<chain::Chain>, distance: u64) -> Vec<SumTreeNode> {
		let mut return_vec = Vec::new();
		let last_n = head.get_last_n_kernel(distance);
		for elem in last_n {
			return_vec.push(SumTreeNode {
				hash: util::to_hex(elem.hash.to_vec()),
				output: None,
			});
		}
		return_vec
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
	/// switch commit hash
	pub switch_commit_hash: Option<core::SwitchCommitHash>,
	/// A proof that the commitment is in the right range
	pub proof: Option<pedersen::RangeProof>,
	/// The height of the block creating this output
	pub height: u64,
	/// The lock height (earliest block this output can be spent)
	pub lock_height: u64,
}

impl Output {
	pub fn from_output(output: &core::Output, block_header: &core::BlockHeader,
		include_proof:bool, include_switch: bool) -> Output {
		let (output_type, lock_height) = match output.features {
			x if x.contains(core::transaction::COINBASE_OUTPUT) => (
				OutputType::Coinbase,
				block_header.height + global::coinbase_maturity(),
			),
			_ => (OutputType::Transaction, 0),
		};

		Output {
			output_type: output_type,
			commit: output.commit,
			switch_commit_hash: match include_switch {
				true => Some(output.switch_commit_hash),
				false => None,
			},
			proof: match include_proof {
				true => Some(output.proof),
				false => None,
			},
			height: block_header.height,
			lock_height: lock_height,
		}
	}
}

// As above, except formatted a bit better for human viewing
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OutputPrintable {
	/// The type of output Coinbase|Transaction
	pub output_type: OutputType,
	/// The homomorphic commitment representing the output's amount (as hex
	/// string)
	pub commit: String,
	/// switch commit hash
	pub switch_commit_hash: String,
	/// The height of the block creating this output
	pub height: u64,
	/// The lock height (earliest block this output can be spent)
	pub lock_height: u64,
	/// Whether the output has been spent
	pub spent: bool,
	/// Rangeproof hash  (as hex string)
	pub proof_hash: Option<String>,
}

impl OutputPrintable {
	pub fn from_output(output: &core::Output, block_header: &core::BlockHeader, include_proof_hash:bool) -> OutputPrintable {
		let (output_type, lock_height) = match output.features {
			x if x.contains(core::transaction::COINBASE_OUTPUT) => (
				OutputType::Coinbase,
				block_header.height + global::coinbase_maturity(),
			),
			_ => (OutputType::Transaction, 0),
		};
		OutputPrintable {
			output_type: output_type,
			commit: util::to_hex(output.commit.0.to_vec()),
			switch_commit_hash: util::to_hex(output.switch_commit_hash.hash.to_vec()),
			height: block_header.height,
			lock_height: lock_height,
			spent: true,
			proof_hash: match include_proof_hash {
				true => Some(util::to_hex(output.proof.hash().to_vec())),
				false => None,
			}
		}
	}
}

// As above, except just the info needed for wallet reconstruction
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OutputSwitch {
	/// the commit
	pub commit: String,
	/// switch commit hash
	pub switch_commit_hash: [u8; core::SWITCH_COMMIT_HASH_SIZE],
	/// The height of the block creating this output
	pub height: u64,
}

impl OutputSwitch {
	pub fn from_output(output: &core::Output, block_header: &core::BlockHeader) -> OutputSwitch {
		OutputSwitch {
			commit: util::to_hex(output.commit.0.to_vec()),
			switch_commit_hash: output.switch_commit_hash.hash,
			height: block_header.height,
		}
	}
}

// Printable representation of a block
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TxKernelPrintable {
	pub features: String,
	pub fee: u64,
	pub lock_height: u64,
	pub excess: String,
	pub excess_sig: String,
}

impl TxKernelPrintable {
	pub fn from_txkernel(k: &core::TxKernel) -> TxKernelPrintable {
		TxKernelPrintable {
			features: format!("{:?}", k.features),
			fee: k.fee,
			lock_height: k.lock_height,
			excess: util::to_hex(k.excess.0.to_vec()),
			excess_sig: util::to_hex(k.excess_sig.to_vec())
		}
	}
}

// Just the information required for wallet reconstruction
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockHeaderInfo {
	// Hash
	pub hash: String,
	/// Height of this block since the genesis block (height 0)
	pub height: u64,
	/// Hash of the block previous to this in the chain.
	pub previous: String,
}

impl BlockHeaderInfo {
	pub fn from_header(h: &core::BlockHeader) -> BlockHeaderInfo {
		BlockHeaderInfo {
			hash: util::to_hex(h.hash().to_vec()),
			height: h.height,
			previous: util::to_hex(h.previous.to_vec()),
		}
	}
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockHeaderPrintable {
	// Hash
	pub hash: String,
	/// Version of the block
	pub version: u16,
	/// Height of this block since the genesis block (height 0)
	pub height: u64,
	/// Hash of the block previous to this in the chain.
	pub previous: String,
	/// rfc3339 timestamp at which the block was built.
	pub timestamp: String,
	/// Merklish root of all the commitments in the UTXO set
	pub utxo_root: String,
	/// Merklish root of all range proofs in the UTXO set
	pub range_proof_root: String,
	/// Merklish root of all transaction kernels in the UTXO set
	pub kernel_root: String,
	/// Nonce increment used to mine this block.
	pub nonce: u64,
	/// Difficulty used to mine the block.
	pub difficulty: u64,
	/// Total accumulated difficulty since genesis block
	pub total_difficulty: u64,
}

impl BlockHeaderPrintable {
	pub fn from_header(h: &core::BlockHeader) -> BlockHeaderPrintable {
		BlockHeaderPrintable {
			hash: util::to_hex(h.hash().to_vec()),
			version: h.version,
			height: h.height,
			previous: util::to_hex(h.previous.to_vec()),
			timestamp: h.timestamp.rfc3339().to_string(),
			utxo_root: util::to_hex(h.utxo_root.to_vec()),
			range_proof_root: util::to_hex(h.range_proof_root.to_vec()),
			kernel_root: util::to_hex(h.kernel_root.to_vec()),
			nonce: h.nonce,
			difficulty: h.difficulty.into_num(),
			total_difficulty: h.total_difficulty.into_num()
		}
	}
}

// Printable representation of a block
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockPrintable {
	/// The block header
	pub header: BlockHeaderPrintable,
	// Input transactions
	pub inputs: Vec<String>,
	/// A printable version of the outputs
	pub outputs: Vec<OutputPrintable>,
	/// A printable version of the transaction kernels
	pub kernels: Vec<TxKernelPrintable>,
}

impl BlockPrintable {
	pub fn from_block(block: &core::Block) -> BlockPrintable {
		let inputs = block.inputs
			.iter()
			.map(|input| util::to_hex((input.0).0.to_vec()))
			.collect();
		let outputs = block.outputs
			.iter()
			.map(|output| OutputPrintable::from_output(output, &block.header, true))
			.collect();
		let kernels = block.kernels
			.iter()
			.map(|kernel| TxKernelPrintable::from_txkernel(kernel))
			.collect();
		BlockPrintable {
			header: BlockHeaderPrintable::from_header(&block.header),
			inputs: inputs,
			outputs: outputs,
			kernels: kernels,
		}
	}
}

// For wallet reconstruction, include the header info along with the
// transactions in the block
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockOutputs {
	/// The block header
	pub header: BlockHeaderInfo,
	/// A printable version of the outputs
	pub outputs: Vec<OutputSwitch>,
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
