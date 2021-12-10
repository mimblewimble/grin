// Copyright 2021 The Grin Developers
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

use crate::chain;
use crate::core::core::hash::Hashed;
use crate::core::core::merkle_proof::MerkleProof;
use crate::core::core::{FeeFields, KernelFeatures, TxKernel};
use crate::core::{core, ser};
use crate::p2p;
use crate::util::secp::pedersen;
use crate::util::{self, ToHex};
use serde::de::MapAccess;
use serde::ser::SerializeStruct;
use std::fmt;

macro_rules! no_dup {
	($field:ident) => {
		if $field.is_some() {
			return Err(serde::de::Error::duplicate_field("$field"));
		}
	};
}

/// API Version Information
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Version {
	/// Current node API Version (api crate version)
	pub node_version: String,
	/// Block header version
	pub block_header_version: u16,
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
			last_block_pushed: tip.last_block_h.to_hex(),
			prev_block_to_last: tip.prev_block_h.to_hex(),
			total_difficulty: tip.total_difficulty.to_num(),
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
	// The current sync status
	pub sync_status: String,
	// Additional sync information
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sync_info: Option<serde_json::Value>,
}

impl Status {
	pub fn from_tip_and_peers(
		current_tip: chain::Tip,
		connections: u32,
		sync_status: String,
		sync_info: Option<serde_json::Value>,
	) -> Status {
		Status {
			protocol_version: ser::ProtocolVersion::local().into(),
			user_agent: p2p::msg::USER_AGENT.to_string(),
			connections: connections,
			tip: Tip::from_tip(current_tip),
			sync_status,
			sync_info,
		}
	}
}

/// TxHashSet
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TxHashSet {
	/// Output Root Hash
	pub output_root_hash: String,
	// Rangeproof root hash
	pub range_proof_root_hash: String,
	// Kernel set root hash
	pub kernel_root_hash: String,
}

impl TxHashSet {
	/// A TxHashSet in the context of the api is simply the collection of PMMR roots.
	/// We can obtain these in a lightweight way by reading them from the head of the chain.
	/// We will have validated the roots on this header against the roots of the txhashset.
	pub fn from_head(chain: &chain::Chain) -> Result<TxHashSet, chain::Error> {
		let header = chain.head_header()?;
		Ok(TxHashSet {
			output_root_hash: header.output_root.to_hex(),
			range_proof_root_hash: header.range_proof_root.to_hex(),
			kernel_root_hash: header.kernel_root.to_hex(),
		})
	}
}

/// Wrapper around a list of txhashset nodes, so it can be
/// presented properly via json
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TxHashSetNode {
	// The hash
	pub hash: String,
}

impl TxHashSetNode {
	pub fn get_last_n_output(chain: &chain::Chain, distance: u64) -> Vec<TxHashSetNode> {
		let mut return_vec = Vec::new();
		let last_n = chain.get_last_n_output(distance);
		for x in last_n {
			return_vec.push(TxHashSetNode { hash: x.0.to_hex() });
		}
		return_vec
	}

	pub fn get_last_n_rangeproof(chain: &chain::Chain, distance: u64) -> Vec<TxHashSetNode> {
		let mut return_vec = Vec::new();
		let last_n = chain.get_last_n_rangeproof(distance);
		for elem in last_n {
			return_vec.push(TxHashSetNode {
				hash: elem.0.to_hex(),
			});
		}
		return_vec
	}

	pub fn get_last_n_kernel(chain: &chain::Chain, distance: u64) -> Vec<TxHashSetNode> {
		let mut return_vec = Vec::new();
		let last_n = chain.get_last_n_kernel(distance);
		for elem in last_n {
			return_vec.push(TxHashSetNode {
				hash: elem.0.to_hex(),
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
	/// The output commitment representing the amount
	pub commit: PrintableCommitment,
	/// Height of the block which contains the output
	pub height: u64,
	/// MMR Index of output
	pub mmr_index: u64,
}

impl Output {
	pub fn new(commit: &pedersen::Commitment, height: u64, mmr_index: u64) -> Output {
		Output {
			commit: PrintableCommitment { commit: *commit },
			height: height,
			mmr_index: mmr_index,
		}
	}
}

#[derive(Debug, Clone)]
pub struct PrintableCommitment {
	pub commit: pedersen::Commitment,
}

impl PrintableCommitment {
	pub fn commit(&self) -> pedersen::Commitment {
		self.commit
	}

	pub fn to_vec(&self) -> Vec<u8> {
		self.commit.0.to_vec()
	}
}

impl AsRef<[u8]> for PrintableCommitment {
	fn as_ref(&self) -> &[u8] {
		&self.commit.0
	}
}

impl serde::ser::Serialize for PrintableCommitment {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::ser::Serializer,
	{
		serializer.serialize_str(&self.to_hex())
	}
}

impl<'de> serde::de::Deserialize<'de> for PrintableCommitment {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::de::Deserializer<'de>,
	{
		deserializer.deserialize_str(PrintableCommitmentVisitor)
	}
}

struct PrintableCommitmentVisitor;

impl<'de> serde::de::Visitor<'de> for PrintableCommitmentVisitor {
	type Value = PrintableCommitment;

	fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str("a Pedersen commitment")
	}

	fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
	where
		E: serde::de::Error,
	{
		Ok(PrintableCommitment {
			commit: pedersen::Commitment::from_vec(
				util::from_hex(v).map_err(serde::de::Error::custom)?,
			),
		})
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
	/// Whether the output has been spent
	pub spent: bool,
	/// Rangeproof (as hex string)
	pub proof: Option<String>,
	/// Rangeproof hash (as hex string)
	pub proof_hash: String,
	/// Block height at which the output is found
	pub block_height: Option<u64>,
	/// Merkle Proof
	pub merkle_proof: Option<MerkleProof>,
	/// MMR Position
	pub mmr_index: u64,
}

impl OutputPrintable {
	pub fn from_output(
		output: &core::Output,
		chain: &chain::Chain,
		block_header: Option<&core::BlockHeader>,
		include_proof: bool,
		include_merkle_proof: bool,
	) -> Result<OutputPrintable, chain::Error> {
		let output_type = if output.is_coinbase() {
			OutputType::Coinbase
		} else {
			OutputType::Transaction
		};

		let pos = chain.get_unspent(output.commitment())?;

		let spent = pos.is_none();

		// If output is unspent then we know its pos and height from the output_pos index.
		// We use the header height directly for spent pos.
		// Note: There is an interesting edge case here and we need to consider if the
		// api is currently doing the right thing here:
		// An output can be spent and then subsequently reused and the new instance unspent.
		// This would result in a height that differs from the provided block height.
		let output_pos = pos.map(|(_, x)| x.pos).unwrap_or(0);
		let block_height = pos
			.map(|(_, x)| x.height)
			.or(block_header.map(|x| x.height));

		let proof = if include_proof {
			Some(output.proof_bytes().to_hex())
		} else {
			None
		};

		// Get the Merkle proof for all unspent coinbase outputs (to verify maturity on
		// spend). We obtain the Merkle proof by rewinding the PMMR.
		// We require the rewind() to be stable even after the PMMR is pruned and
		// compacted so we can still recreate the necessary proof.
		let mut merkle_proof = None;
		if include_merkle_proof && output.is_coinbase() && !spent {
			if let Some(block_header) = block_header {
				merkle_proof = chain.get_merkle_proof(output, &block_header).ok();
			}
		};

		Ok(OutputPrintable {
			output_type,
			commit: output.commitment(),
			spent,
			proof,
			proof_hash: output.proof.hash().to_hex(),
			block_height,
			merkle_proof,
			mmr_index: output_pos,
		})
	}

	pub fn commit(&self) -> Result<pedersen::Commitment, ser::Error> {
		Ok(self.commit)
	}

	pub fn range_proof(&self) -> Result<pedersen::RangeProof, ser::Error> {
		let proof_str = self
			.proof
			.clone()
			.ok_or_else(|| ser::Error::HexError("output range_proof missing".to_string()))?;

		let p_vec = util::from_hex(&proof_str)
			.map_err(|_| ser::Error::HexError("invalid output range_proof".to_string()))?;
		let mut p_bytes = [0; util::secp::constants::MAX_PROOF_SIZE];
		p_bytes.clone_from_slice(&p_vec[..util::secp::constants::MAX_PROOF_SIZE]);
		Ok(pedersen::RangeProof {
			proof: p_bytes,
			plen: p_bytes.len(),
		})
	}
}

impl serde::ser::Serialize for OutputPrintable {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::ser::Serializer,
	{
		let mut state = serializer.serialize_struct("OutputPrintable", 7)?;
		state.serialize_field("output_type", &self.output_type)?;
		state.serialize_field("commit", &self.commit.to_hex())?;
		state.serialize_field("spent", &self.spent)?;
		state.serialize_field("proof", &self.proof)?;
		state.serialize_field("proof_hash", &self.proof_hash)?;
		state.serialize_field("block_height", &self.block_height)?;

		let hex_merkle_proof = &self.merkle_proof.clone().map(|x| x.to_hex());
		state.serialize_field("merkle_proof", &hex_merkle_proof)?;
		state.serialize_field("mmr_index", &self.mmr_index)?;

		state.end()
	}
}

impl<'de> serde::de::Deserialize<'de> for OutputPrintable {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::de::Deserializer<'de>,
	{
		#[derive(Deserialize)]
		#[serde(field_identifier, rename_all = "snake_case")]
		enum Field {
			OutputType,
			Commit,
			Spent,
			Proof,
			ProofHash,
			BlockHeight,
			MerkleProof,
			MmrIndex,
		}

		struct OutputPrintableVisitor;

		impl<'de> serde::de::Visitor<'de> for OutputPrintableVisitor {
			type Value = OutputPrintable;

			fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
				formatter.write_str("a print able Output")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: MapAccess<'de>,
			{
				let mut output_type = None;
				let mut commit = None;
				let mut spent = None;
				let mut proof = None;
				let mut proof_hash = None;
				let mut block_height = None;
				let mut merkle_proof = None;
				let mut mmr_index = None;

				while let Some(key) = map.next_key()? {
					match key {
						Field::OutputType => {
							no_dup!(output_type);
							output_type = Some(map.next_value()?)
						}
						Field::Commit => {
							no_dup!(commit);

							let val: String = map.next_value()?;
							let vec = util::from_hex(&val).map_err(serde::de::Error::custom)?;
							commit = Some(pedersen::Commitment::from_vec(vec));
						}
						Field::Spent => {
							no_dup!(spent);
							spent = Some(map.next_value()?)
						}
						Field::Proof => {
							no_dup!(proof);
							proof = map.next_value()?
						}
						Field::ProofHash => {
							no_dup!(proof_hash);
							proof_hash = Some(map.next_value()?)
						}
						Field::BlockHeight => {
							no_dup!(block_height);
							block_height = Some(map.next_value()?)
						}
						Field::MerkleProof => {
							no_dup!(merkle_proof);
							if let Some(hex) = map.next_value::<Option<String>>()? {
								if let Ok(res) = MerkleProof::from_hex(&hex) {
									merkle_proof = Some(res);
								} else {
									merkle_proof = Some(MerkleProof::empty());
								}
							}
						}
						Field::MmrIndex => {
							no_dup!(mmr_index);
							mmr_index = Some(map.next_value()?)
						}
					}
				}

				if output_type.is_none()
					|| commit.is_none() || spent.is_none()
					|| proof_hash.is_none()
					|| mmr_index.is_none()
				{
					return Err(serde::de::Error::custom("invalid output"));
				}

				Ok(OutputPrintable {
					output_type: output_type.unwrap(),
					commit: commit.unwrap(),
					spent: spent.unwrap(),
					proof: proof,
					proof_hash: proof_hash.unwrap(),
					block_height: block_height.unwrap(),
					merkle_proof: merkle_proof,
					mmr_index: mmr_index.unwrap(),
				})
			}
		}

		const FIELDS: &[&str] = &[
			"output_type",
			"commit",
			"spent",
			"proof",
			"proof_hash",
			"mmr_index",
		];
		deserializer.deserialize_struct("OutputPrintable", FIELDS, OutputPrintableVisitor)
	}
}

// Printable representation of a block
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TxKernelPrintable {
	pub features: String,
	pub fee_shift: u8,
	pub fee: u64,
	pub lock_height: u64,
	pub excess: String,
	pub excess_sig: String,
}

impl TxKernelPrintable {
	pub fn from_txkernel(k: &core::TxKernel) -> TxKernelPrintable {
		let features = k.features.as_string();
		let (fee_fields, lock_height) = match k.features {
			KernelFeatures::Plain { fee } => (fee, 0),
			KernelFeatures::Coinbase => (FeeFields::zero(), 0),
			KernelFeatures::HeightLocked { fee, lock_height } => (fee, lock_height),
			KernelFeatures::NoRecentDuplicate {
				fee,
				relative_height,
			} => (fee, relative_height.into()),
		};
		TxKernelPrintable {
			features,
			fee_shift: fee_fields.fee_shift(),
			fee: fee_fields.fee(),
			lock_height,
			excess: k.excess.to_hex(),
			excess_sig: (&k.excess_sig.to_raw_data()[..]).to_hex(),
		}
	}
}

// Just the information required for wallet reconstruction
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockHeaderDifficultyInfo {
	// Hash
	pub hash: String,
	/// Height of this block since the genesis block (height 0)
	pub height: u64,
	/// Hash of the block previous to this in the chain.
	pub previous: String,
}

impl BlockHeaderDifficultyInfo {
	pub fn from_header(header: &core::BlockHeader) -> BlockHeaderDifficultyInfo {
		BlockHeaderDifficultyInfo {
			hash: header.hash().to_hex(),
			height: header.height,
			previous: header.prev_hash.to_hex(),
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
	/// Root hash of the header MMR at the previous header.
	pub prev_root: String,
	/// rfc3339 timestamp at which the block was built.
	pub timestamp: String,
	/// Merklish root of all the commitments in the TxHashSet
	pub output_root: String,
	/// Size of the output MMR
	pub output_mmr_size: u64,
	/// Merklish root of all range proofs in the TxHashSet
	pub range_proof_root: String,
	/// Merklish root of all transaction kernels in the TxHashSet
	pub kernel_root: String,
	/// Size of the kernel MMR
	pub kernel_mmr_size: u64,
	/// Nonce increment used to mine this block.
	pub nonce: u64,
	/// Size of the cuckoo graph
	pub edge_bits: u8,
	/// Nonces of the cuckoo solution
	pub cuckoo_solution: Vec<u64>,
	/// Total accumulated difficulty since genesis block
	pub total_difficulty: u64,
	/// Variable difficulty scaling factor for secondary proof of work
	pub secondary_scaling: u32,
	/// Total kernel offset since genesis block
	pub total_kernel_offset: String,
}

impl BlockHeaderPrintable {
	pub fn from_header(header: &core::BlockHeader) -> BlockHeaderPrintable {
		BlockHeaderPrintable {
			hash: header.hash().to_hex(),
			version: header.version.into(),
			height: header.height,
			previous: header.prev_hash.to_hex(),
			prev_root: header.prev_root.to_hex(),
			timestamp: header.timestamp.to_rfc3339(),
			output_root: header.output_root.to_hex(),
			output_mmr_size: header.output_mmr_size,
			range_proof_root: header.range_proof_root.to_hex(),
			kernel_root: header.kernel_root.to_hex(),
			kernel_mmr_size: header.kernel_mmr_size,
			nonce: header.pow.nonce,
			edge_bits: header.pow.edge_bits(),
			cuckoo_solution: header.pow.proof.nonces.clone(),
			total_difficulty: header.pow.total_difficulty.to_num(),
			secondary_scaling: header.pow.secondary_scaling,
			total_kernel_offset: header.total_kernel_offset.to_hex(),
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
		chain: &chain::Chain,
		include_proof: bool,
		include_merkle_proof: bool,
	) -> Result<BlockPrintable, chain::Error> {
		let inputs: Vec<_> = block.inputs().into();
		let inputs = inputs.iter().map(|x| x.commitment().to_hex()).collect();
		let outputs = block
			.outputs()
			.iter()
			.map(|output| {
				OutputPrintable::from_output(
					output,
					chain,
					Some(&block.header),
					include_proof,
					include_merkle_proof,
				)
			})
			.collect::<Result<Vec<_>, _>>()?;

		let kernels = block
			.kernels()
			.iter()
			.map(|kernel| TxKernelPrintable::from_txkernel(kernel))
			.collect();
		Ok(BlockPrintable {
			header: BlockHeaderPrintable::from_header(&block.header),
			inputs: inputs,
			outputs: outputs,
			kernels: kernels,
		})
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
	/// Convert a compact block into a printable representation suitable for
	/// api response
	pub fn from_compact_block(
		cb: &core::CompactBlock,
		chain: &chain::Chain,
	) -> Result<CompactBlockPrintable, chain::Error> {
		let block = chain.get_block(&cb.hash())?;
		let out_full = cb
			.out_full()
			.iter()
			.map(|x| OutputPrintable::from_output(x, chain, Some(&block.header), false, true))
			.collect::<Result<Vec<_>, _>>()?;
		let kern_full = cb
			.kern_full()
			.iter()
			.map(|x| TxKernelPrintable::from_txkernel(x))
			.collect();
		Ok(CompactBlockPrintable {
			header: BlockHeaderPrintable::from_header(&cb.header),
			out_full,
			kern_full,
			kern_ids: cb.kern_ids().iter().map(|x| x.to_hex()).collect(),
		})
	}
}

// For wallet reconstruction, include the header info along with the
// transactions in the block
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockOutputs {
	/// The block header
	pub header: BlockHeaderDifficultyInfo,
	/// A printable version of the outputs
	pub outputs: Vec<OutputPrintable>,
}

// For traversing all outputs in the UTXO set
// transactions in the block
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OutputListing {
	/// The last available output index
	pub highest_index: u64,
	/// The last insertion index retrieved
	pub last_retrieved_index: u64,
	/// A printable version of the outputs
	pub outputs: Vec<OutputPrintable>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LocatedTxKernel {
	pub tx_kernel: TxKernel,
	pub height: u64,
	pub mmr_index: u64,
}

#[derive(Serialize, Deserialize)]
pub struct PoolInfo {
	/// Size of the pool
	pub pool_size: usize,
}

#[cfg(test)]
mod test {
	use super::*;
	use serde_json;

	#[test]
	fn serialize_output_printable() {
		let hex_output = "{\
			 \"output_type\":\"Coinbase\",\
			 \"commit\":\"083eafae5d61a85ab07b12e1a51b3918d8e6de11fc6cde641d54af53608aa77b9f\",\
			 \"spent\":false,\
			 \"proof\":null,\
			 \"proof_hash\":\"ed6ba96009b86173bade6a9227ed60422916593fa32dd6d78b25b7a4eeef4946\",\
			 \"block_height\":0,\
			 \"merkle_proof\":null,\
			 \"mmr_index\":0\
			 }";
		let deserialized: OutputPrintable = serde_json::from_str(&hex_output).unwrap();
		let serialized = serde_json::to_string(&deserialized).unwrap();
		assert_eq!(serialized, hex_output);
	}

	#[test]
	fn serialize_output() {
		let hex_commit = "{\
			 \"commit\":\"083eafae5d61a85ab07b12e1a51b3918d8e6de11fc6cde641d54af53608aa77b9f\",\
			 \"height\":0,\
			 \"mmr_index\":0\
			 }";
		let deserialized: Output = serde_json::from_str(&hex_commit).unwrap();
		let serialized = serde_json::to_string(&deserialized).unwrap();
		assert_eq!(serialized, hex_commit);
	}
}
