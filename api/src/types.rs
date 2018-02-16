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
use core::core::SwitchCommitHash;
use chain;
use p2p;
use util;
use util::secp::pedersen;
use util::secp::constants::MAX_PROOF_SIZE;
use serde;
use serde::ser::SerializeStruct;
use serde::de::MapAccess;
use std::fmt;

macro_rules! no_dup {
	($field: ident) => {
		if $field.is_some() {
			return Err(serde::de::Error::duplicate_field("$field"));
		}
	};
}

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
	pub commit: PrintableCommitment,
}

impl Utxo {
	pub fn new(commit: &pedersen::Commitment) -> Utxo {
		Utxo { commit: PrintableCommitment(commit.clone()) }
	}
}

#[derive(Debug, Clone)]
pub struct PrintableCommitment(pedersen::Commitment);

impl PrintableCommitment {
	pub fn commit(&self) -> pedersen::Commitment {
		self.0.clone()
	}

	pub fn to_vec(&self) -> Vec<u8> {
		let commit = self.0;
		commit.0.to_vec()
	}
}

impl serde::ser::Serialize for PrintableCommitment {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where
		S: serde::ser::Serializer {
		serializer.serialize_str(&util::to_hex(self.to_vec()))
	}
}

impl<'de> serde::de::Deserialize<'de> for PrintableCommitment {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where
		D: serde::de::Deserializer<'de> {
		deserializer.deserialize_str(PrintableCommitmentVisitor)
	}
}

struct PrintableCommitmentVisitor;

impl<'de> serde::de::Visitor<'de> for PrintableCommitmentVisitor {
	type Value = PrintableCommitment;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("a Pedersen commitment")
	}

	fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> where
		E: serde::de::Error, {
		Ok(PrintableCommitment(pedersen::Commitment::from_vec(util::from_hex(String::from(v)).unwrap())))
	}
}

// As above, except formatted a bit better for human viewing
#[derive(Debug, Clone)]
pub struct OutputPrintable {
	/// The type of output Coinbase|Transaction
	pub output_type: OutputType,
	/// The homomorphic commitment representing the output's amount
	/// (as hex string)
	pub commit: pedersen::Commitment,
	/// switch commit hash
	pub switch_commit_hash: SwitchCommitHash,
	/// Whether the output has been spent
	pub spent: bool,
	/// Rangeproof (as hex string)
	pub proof: Option<pedersen::RangeProof>,
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
			if output.features.contains(core::transaction::OutputFeatures::COINBASE_OUTPUT) {
				OutputType::Coinbase
			} else {
				OutputType::Transaction
			};

		let out_id = core::OutputIdentifier::from_output(&output);
		let spent = chain.is_unspent(&out_id).is_err();

		let proof = if include_proof {
			Some(output.proof)
		} else {
			None
		};

		OutputPrintable {
			output_type,
			commit: output.commit,
			switch_commit_hash: output.switch_commit_hash,
			spent,
			proof,
			proof_hash: util::to_hex(output.proof.hash().to_vec()),
		}
	}

	// Convert the hex string back into a switch_commit_hash instance
	pub fn switch_commit_hash(&self) -> Result<core::SwitchCommitHash, ser::Error> {
		Ok(self.switch_commit_hash.clone())
	}

	pub fn commit(&self) -> Result<pedersen::Commitment, ser::Error> {
		Ok(self.commit.clone())
	}

	pub fn range_proof(&self) -> Result<pedersen::RangeProof, ser::Error> {
		self.proof.clone().ok_or_else(|| ser::Error::HexError(format!("output range_proof missing")))
	}
}

impl serde::ser::Serialize for OutputPrintable {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where
		S: serde::ser::Serializer {
		let mut state = serializer.serialize_struct("OutputPrintable", 6)?;
		state.serialize_field("output_type", &self.output_type)?;
		state.serialize_field("commit", &util::to_hex(self.commit.0.to_vec()))?;
		state.serialize_field("switch_commit_hash", &self.switch_commit_hash.to_hex())?;
		state.serialize_field("spent", &self.spent)?;
		state.serialize_field("proof", &self.proof)?;
		state.serialize_field("proof_hash", &self.proof_hash)?;
		state.end()
	}
}

impl<'de> serde::de::Deserialize<'de> for OutputPrintable {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where
		D: serde::de::Deserializer<'de> {
		#[derive(Deserialize)]
		#[serde(field_identifier, rename_all = "snake_case")]
		enum Field {
			OutputType,
			Commit,
			SwitchCommitHash,
			Spent,
			Proof,
			ProofHash
		}

		struct OutputPrintableVisitor;

		impl<'de> serde::de::Visitor<'de> for OutputPrintableVisitor {
			type Value = OutputPrintable;

			fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
				formatter.write_str("a print able Output")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error> where
				A: MapAccess<'de>, {
				let mut output_type = None;
				let mut commit = None;
				let mut switch_commit_hash = None;
				let mut spent = None;
				let mut proof = None;
				let mut proof_hash = None;

				while let Some(key) = map.next_key()? {
					match key {
						Field::OutputType => {
							no_dup!(output_type);
							output_type = Some(map.next_value()?)
						},
						Field::Commit => {
							no_dup!(commit);

							let val: String = map.next_value()?;
							let vec = util::from_hex(val.clone())
								.map_err(serde::de::Error::custom)?;
							commit = Some(pedersen::Commitment::from_vec(vec));
						},
						Field::SwitchCommitHash => {
							no_dup!(switch_commit_hash);

							let val: String = map.next_value()?;
							let hash = core::SwitchCommitHash::from_hex(&val.clone())
								.map_err(serde::de::Error::custom)?;
							switch_commit_hash = Some(hash)
						},
						Field::Spent => {
							no_dup!(spent);
							spent = Some(map.next_value()?)
						},
						Field::Proof => {
							no_dup!(proof);

							let val: Option<String> = map.next_value()?;

							if val.is_some() {
								let vec = util::from_hex(val.unwrap().clone())
									.map_err(serde::de::Error::custom)?;
								let mut bytes = [0; MAX_PROOF_SIZE];
								for i in 0..vec.len() {
									bytes[i] = vec[i];
								}

								proof = Some(pedersen::RangeProof { proof: bytes, plen: vec.len() })
							}
						},
						Field::ProofHash => {
							no_dup!(proof_hash);
							proof_hash = Some(map.next_value()?)
						}
					}
				}

				Ok(OutputPrintable {
					output_type: output_type.unwrap(),
					commit: commit.unwrap(),
					switch_commit_hash: switch_commit_hash.unwrap(),
					spent: spent.unwrap(),
					proof: proof,
					proof_hash: proof_hash.unwrap()
				})
			}
		}

		const FIELDS: &'static [&'static str] = &["output_type", "commit", "switch_commit_hash", "spent", "proof", "proof_hash"];
		deserializer.deserialize_struct("OutputPrintable", FIELDS, OutputPrintableVisitor)
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
	/// Full outputs, specifically coinbase output(s)
	pub out_full: Vec<OutputPrintable>,
	/// Full kernels, specifically coinbase kernel(s)
	pub kern_full: Vec<TxKernelPrintable>,
	/// Kernels (hex short_ids)
	pub kern_ids: Vec<String>,
}

impl CompactBlockPrintable {
	/// Convert a compact block into a printable representation suitable for api response
	pub fn from_compact_block(
		cb: &core::CompactBlock,
		chain: Arc<chain::Chain>,
	) -> CompactBlockPrintable {
		let out_full = cb
			.out_full
			.iter()
			.map(|x| OutputPrintable::from_output(x, chain.clone(), false))
			.collect();
		let kern_full = cb
			.kern_full
			.iter()
			.map(|x| TxKernelPrintable::from_txkernel(x))
			.collect();
		CompactBlockPrintable {
			header: BlockHeaderPrintable::from_header(&cb.header),
			out_full,
			kern_full,
			kern_ids: cb.kern_ids.iter().map(|x| x.to_hex()).collect(),
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

#[cfg(test)]
mod test {
	use super::*;
	use serde_json;

	#[test]
	fn serialize_output() {
		let hex_output = "{\
			\"output_type\":\"Coinbase\",\
			\"commit\":\"083eafae5d61a85ab07b12e1a51b3918d8e6de11fc6cde641d54af53608aa77b9f\",\
			\"switch_commit_hash\":\"85daaf11011dc11e52af84ebe78e2f2d19cbdc76000000000000000000000000\",\
			\"spent\":false,\
			\"proof\":null,\
			\"proof_hash\":\"ed6ba96009b86173bade6a9227ed60422916593fa32dd6d78b25b7a4eeef4946\"\
		}";
		let deserialized: OutputPrintable = serde_json::from_str(&hex_output).unwrap();
		let serialized = serde_json::to_string(&deserialized).unwrap();
		assert_eq!(serialized, hex_output);
	}

	#[test]
	fn serialize_utxo() {
		let hex_commit = "{\"commit\":\"083eafae5d61a85ab07b12e1a51b3918d8e6de11fc6cde641d54af53608aa77b9f\"}";
		let deserialized: Utxo = serde_json::from_str(&hex_commit).unwrap();
		let serialized = serde_json::to_string(&deserialized).unwrap();
		assert_eq!(serialized, hex_commit);
	}
}
