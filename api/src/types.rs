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

use core::{core, ser};
use core::core::hash::Hashed;
use core::core::SumCommit;
use chain;
use p2p;
use util;
use util::secp::pedersen;
use util::secp::constants::MAX_PROOF_SIZE;

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

/// Status page containing different server information
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Status {
	// The protocol version
	pub protocol_version: u32,
	// The user user agent
	pub user_agent: String,
	// The current number of connections
	pub connections: u32,
	// The state of the current fork Tip
	pub tip: Tip,
}

impl Status {
	pub fn from_tip_and_peers(current_tip: chain::Tip, connections: u32) -> Status {
		Status {
			protocol_version: p2p::msg::PROTOCOL_VERSION,
			user_agent: p2p::msg::USER_AGENT.to_string(),
			connections: connections,
			tip: Tip::from_tip(current_tip),
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
			utxo_root_hash: roots.0.hash.to_hex(),
			utxo_root_sum: roots.0.sum.to_hex(),
			range_proof_root_hash: roots.1.hash.to_hex(),
			kernel_root_hash: roots.2.hash.to_hex(),
		}
	}
}

/// Wrapper around a list of sumtree nodes, so it can be
/// presented properly via json
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SumTreeNode {
	// The hash
	pub hash: String,
	// SumCommit (features|commitment), optional (only for utxos)
	pub sum: Option<SumCommit>,
}

impl SumTreeNode {
	pub fn get_last_n_utxo(chain: Arc<chain::Chain>, distance: u64) -> Vec<SumTreeNode> {
		let mut return_vec = Vec::new();
		let last_n = chain.get_last_n_utxo(distance);
		for x in last_n {
			return_vec.push(SumTreeNode {
				hash: util::to_hex(x.hash.to_vec()),
				sum: Some(x.sum),
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
				sum: None,
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
				sum: None,
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
pub struct Utxo {
	/// The output commitment representing the amount
	pub commit: pedersen::Commitment,
}

impl Utxo {
	pub fn new(commit: &pedersen::Commitment) -> Utxo {
		Utxo { commit: commit.clone() }
	}
}

// As above, except formatted a bit better for human viewing
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OutputPrintable {
	/// The type of output Coinbase|Transaction
	pub output_type: OutputType,
	/// The homomorphic commitment representing the output's amount
	/// (as hex string)
	pub commit: String,
	/// switch commit hash
	pub switch_commit_hash: String,
	/// Whether the output has been spent
	pub spent: bool,
	/// Rangeproof (as hex string)
	pub proof: Option<String>,
	/// Rangeproof hash (as hex string)
	pub proof_hash: String,
}

impl OutputPrintable {
	pub fn from_output(
		output: &core::Output,
		chain: Arc<chain::Chain>,
		include_proof: bool,
	) -> OutputPrintable {
		let output_type =
			if output.features.contains(core::transaction::COINBASE_OUTPUT) {
				OutputType::Coinbase
			} else {
				OutputType::Transaction
			};

		let out_id = core::OutputIdentifier::from_output(&output);
		let spent = chain.is_unspent(&out_id).is_err();

		let proof = if include_proof {
			Some(util::to_hex(output.proof.bytes().to_vec()))
		} else {
			None
		};

		OutputPrintable {
			output_type: output_type,
			commit: util::to_hex(output.commit.0.to_vec()),
			switch_commit_hash: output.switch_commit_hash.to_hex(),
			spent: spent,
			proof: proof,
			proof_hash: util::to_hex(output.proof.hash().to_vec()),
		}
	}

	// Convert the hex string back into a switch_commit_hash instance
	pub fn switch_commit_hash(&self) -> Result<core::SwitchCommitHash, ser::Error> {
		core::SwitchCommitHash::from_hex(&self.switch_commit_hash)
	}

	pub fn commit(&self) -> Result<pedersen::Commitment, ser::Error> {
		let vec = util::from_hex(self.commit.clone())
			.map_err(|_| ser::Error::HexError(format!("output commit hex_error")))?;
		Ok(pedersen::Commitment::from_vec(vec))
	}

	pub fn range_proof(&self) -> Result<pedersen::RangeProof, ser::Error> {
		if let Some(ref proof) = self.proof {
			let vec = util::from_hex(proof.clone())
				.map_err(|_| ser::Error::HexError(format!("output range_proof hex_error")))?;
			let mut bytes = [0; MAX_PROOF_SIZE];
			for i in 0..vec.len() {
				bytes[i] = vec[i];
			}
			Ok(pedersen::RangeProof { proof: bytes, plen: vec.len() })
		} else {
			Err(ser::Error::HexError(format!("output range_proof missing")))
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
			excess_sig: util::to_hex(k.excess_sig.to_raw_data().to_vec()),
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
			total_difficulty: h.total_difficulty.into_num(),
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
	pub fn from_block(
		block: &core::Block,
		chain: Arc<chain::Chain>,
		include_proof: bool,
	) -> BlockPrintable {
		let inputs = block.inputs
			.iter()
			.map(|x| util::to_hex(x.commitment().0.to_vec()))
			.collect();
		let outputs = block
			.outputs
			.iter()
			.map(|output| OutputPrintable::from_output(output, chain.clone(), include_proof))
			.collect();
		let kernels = block
			.kernels
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CompactBlockPrintable {
	/// The block header
	pub header: BlockHeaderPrintable,
	/// Inputs (hex short_ids)
	pub inputs: Vec<String>,
	/// Outputs (hex short_ids)
	pub outputs: Vec<String>,
	/// Kernels (hex short_ids)
	pub kernels: Vec<String>,
}

impl CompactBlockPrintable {
	/// Convert a compact block into a printable representation suitable for api response
	pub fn from_compact_block(cb: &core::CompactBlock) -> CompactBlockPrintable {
		CompactBlockPrintable {
			header: BlockHeaderPrintable::from_header(&cb.header),
			inputs: cb.inputs.iter().map(|x| x.to_hex()).collect(),
			outputs: cb.outputs.iter().map(|x| x.to_hex()).collect(),
			kernels: cb.kernels.iter().map(|x| x.to_hex()).collect(),
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
	pub outputs: Vec<OutputPrintable>,
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
