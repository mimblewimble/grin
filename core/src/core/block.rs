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

//! Blocks and blockheaders

use chrono::naive::{MAX_DATE, MIN_DATE};
use chrono::prelude::{DateTime, NaiveDateTime, Utc};
use std::collections::HashSet;
use std::fmt;
use std::iter::FromIterator;
use std::mem;
use std::sync::Arc;
use util::RwLock;

use consensus::{self, reward, REWARD};
use core::committed::{self, Committed};
use core::compact_block::{CompactBlock, CompactBlockBody};
use core::hash::{Hash, Hashed, ZERO_HASH};
use core::verifier_cache::VerifierCache;
use core::{
	transaction, Commitment, Input, KernelFeatures, Output, OutputFeatures, Transaction,
	TransactionBody, TxKernel,
};
use global;
use keychain::{self, BlindingFactor};
use pow::{Difficulty, Proof, ProofOfWork};
use ser::{self, PMMRable, Readable, Reader, Writeable, Writer};
use util::{secp, static_secp_instance};

/// Errors thrown by Block validation
#[derive(Debug, Clone, Eq, PartialEq, Fail)]
pub enum Error {
	/// The sum of output minus input commitments does not
	/// match the sum of kernel commitments
	KernelSumMismatch,
	/// The total kernel sum on the block header is wrong
	InvalidTotalKernelSum,
	/// Same as above but for the coinbase part of a block, including reward
	CoinbaseSumMismatch,
	/// Restrict block total weight.
	TooHeavy,
	/// Block weight (based on inputs|outputs|kernels) exceeded.
	WeightExceeded,
	/// Kernel not valid due to lock_height exceeding block header height
	KernelLockHeight(u64),
	/// Underlying tx related error
	Transaction(transaction::Error),
	/// Underlying Secp256k1 error (signature validation or invalid public key
	/// typically)
	Secp(secp::Error),
	/// Underlying keychain related error
	Keychain(keychain::Error),
	/// Underlying consensus error (sort order currently)
	Consensus(consensus::Error),
	/// Underlying Merkle proof error
	MerkleProof,
	/// Error when verifying kernel sums via committed trait.
	Committed(committed::Error),
	/// Validation error relating to cut-through.
	/// Specifically the tx is spending its own output, which is not valid.
	CutThrough,
	/// Other unspecified error condition
	Other(String),
}

impl From<committed::Error> for Error {
	fn from(e: committed::Error) -> Error {
		Error::Committed(e)
	}
}

impl From<transaction::Error> for Error {
	fn from(e: transaction::Error) -> Error {
		Error::Transaction(e)
	}
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<keychain::Error> for Error {
	fn from(e: keychain::Error) -> Error {
		Error::Keychain(e)
	}
}

impl From<consensus::Error> for Error {
	fn from(e: consensus::Error) -> Error {
		Error::Consensus(e)
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Block Error (display needs implementation")
	}
}

/// Block header, fairly standard compared to other blockchains.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockHeader {
	/// Version of the block
	pub version: u16,
	/// Height of this block since the genesis block (height 0)
	pub height: u64,
	/// Hash of the block previous to this in the chain.
	pub previous: Hash,
	/// Root hash of the header MMR at the previous header.
	pub prev_root: Hash,
	/// Timestamp at which the block was built.
	pub timestamp: DateTime<Utc>,
	/// Merklish root of all the commitments in the TxHashSet
	pub output_root: Hash,
	/// Merklish root of all range proofs in the TxHashSet
	pub range_proof_root: Hash,
	/// Merklish root of all transaction kernels in the TxHashSet
	pub kernel_root: Hash,
	/// Total accumulated sum of kernel offsets since genesis block.
	/// We can derive the kernel offset sum for *this* block from
	/// the total kernel offset of the previous block header.
	pub total_kernel_offset: BlindingFactor,
	/// Total size of the output MMR after applying this block
	pub output_mmr_size: u64,
	/// Total size of the kernel MMR after applying this block
	pub kernel_mmr_size: u64,
	/// Proof of work and related
	pub pow: ProofOfWork,
}

/// Serialized size of fixed part of a BlockHeader, i.e. without pow
fn fixed_size_of_serialized_header(_version: u16) -> usize {
	let mut size: usize = 0;
	size += mem::size_of::<u16>(); // version
	size += mem::size_of::<u64>(); // height
	size += mem::size_of::<i64>(); // timestamp
	size += mem::size_of::<Hash>(); // previous
	size += mem::size_of::<Hash>(); // prev_root
	size += mem::size_of::<Hash>(); // output_root
	size += mem::size_of::<Hash>(); // range_proof_root
	size += mem::size_of::<Hash>(); // kernel_root
	size += mem::size_of::<BlindingFactor>(); // total_kernel_offset
	size += mem::size_of::<u64>(); // output_mmr_size
	size += mem::size_of::<u64>(); // kernel_mmr_size
	size += mem::size_of::<Difficulty>(); // total_difficulty
	size += mem::size_of::<u32>(); // secondary_scaling
	size += mem::size_of::<u64>(); // nonce
	size
}

/// Serialized size of a BlockHeader
pub fn serialized_size_of_header(version: u16, edge_bits: u8) -> usize {
	let mut size = fixed_size_of_serialized_header(version);

	size += mem::size_of::<u8>(); // pow.edge_bits
	let nonce_bits = edge_bits as usize;
	let bitvec_len = global::proofsize() * nonce_bits;
	size += bitvec_len / 8; // pow.nonces
	if bitvec_len % 8 != 0 {
		size += 1;
	}
	size
}

impl Default for BlockHeader {
	fn default() -> BlockHeader {
		BlockHeader {
			version: 1,
			height: 0,
			timestamp: DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(0, 0), Utc),
			previous: ZERO_HASH,
			prev_root: ZERO_HASH,
			output_root: ZERO_HASH,
			range_proof_root: ZERO_HASH,
			kernel_root: ZERO_HASH,
			total_kernel_offset: BlindingFactor::zero(),
			output_mmr_size: 0,
			kernel_mmr_size: 0,
			pow: ProofOfWork::default(),
		}
	}
}

/// Block header hashes are maintained in the header MMR
/// but we store the data itself in the db.
impl PMMRable for BlockHeader {
	fn len() -> usize {
		0
	}
}

/// Serialization of a block header
impl Writeable for BlockHeader {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		if writer.serialization_mode() != ser::SerializationMode::Hash {
			self.write_pre_pow(writer)?;
		}
		self.pow.write(self.version, writer)?;
		Ok(())
	}
}

/// Deserialization of a block header
impl Readable for BlockHeader {
	fn read(reader: &mut Reader) -> Result<BlockHeader, ser::Error> {
		let (version, height, timestamp) = ser_multiread!(reader, read_u16, read_u64, read_i64);
		let previous = Hash::read(reader)?;
		let prev_root = Hash::read(reader)?;
		let output_root = Hash::read(reader)?;
		let range_proof_root = Hash::read(reader)?;
		let kernel_root = Hash::read(reader)?;
		let total_kernel_offset = BlindingFactor::read(reader)?;
		let (output_mmr_size, kernel_mmr_size) = ser_multiread!(reader, read_u64, read_u64);
		let pow = ProofOfWork::read(version, reader)?;

		if timestamp > MAX_DATE.and_hms(0, 0, 0).timestamp()
			|| timestamp < MIN_DATE.and_hms(0, 0, 0).timestamp()
		{
			return Err(ser::Error::CorruptedData);
		}

		Ok(BlockHeader {
			version,
			height,
			timestamp: DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(timestamp, 0), Utc),
			previous,
			prev_root,
			output_root,
			range_proof_root,
			kernel_root,
			total_kernel_offset,
			output_mmr_size,
			kernel_mmr_size,
			pow,
		})
	}
}

impl BlockHeader {
	/// Write the pre-hash portion of the header
	pub fn write_pre_pow<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u16, self.version],
			[write_u64, self.height],
			[write_i64, self.timestamp.timestamp()],
			[write_fixed_bytes, &self.previous],
			[write_fixed_bytes, &self.prev_root],
			[write_fixed_bytes, &self.output_root],
			[write_fixed_bytes, &self.range_proof_root],
			[write_fixed_bytes, &self.kernel_root],
			[write_fixed_bytes, &self.total_kernel_offset],
			[write_u64, self.output_mmr_size],
			[write_u64, self.kernel_mmr_size]
		);
		Ok(())
	}

	/// Return the pre-pow, unhashed
	/// Let the cuck(at)oo miner/verifier handle the hashing
	/// for consistency with how this call is performed everywhere
	/// else
	pub fn pre_pow(&self) -> Vec<u8> {
		let mut header_buf = vec![];
		{
			let mut writer = ser::BinWriter::new(&mut header_buf);
			self.write_pre_pow(&mut writer).unwrap();
			self.pow.write_pre_pow(self.version, &mut writer).unwrap();
			writer.write_u64(self.pow.nonce).unwrap();
		}
		header_buf
	}

	/// Total difficulty accumulated by the proof of work on this header
	pub fn total_difficulty(&self) -> Difficulty {
		self.pow.total_difficulty
	}

	/// The "overage" to use when verifying the kernel sums.
	/// For a block header the overage is 0 - reward.
	pub fn overage(&self) -> i64 {
		(REWARD as i64).checked_neg().unwrap_or(0)
	}

	/// The "total overage" to use when verifying the kernel sums for a full
	/// chain state. For a full chain state this is 0 - (height * reward).
	pub fn total_overage(&self) -> i64 {
		((self.height * REWARD) as i64).checked_neg().unwrap_or(0)
	}

	/// Total kernel offset for the chain state up to and including this block.
	pub fn total_kernel_offset(&self) -> BlindingFactor {
		self.total_kernel_offset
	}

	/// Serialized size of this header
	pub fn serialized_size(&self) -> usize {
		let mut size = fixed_size_of_serialized_header(self.version);

		size += mem::size_of::<u8>(); // pow.edge_bits
		let nonce_bits = self.pow.edge_bits() as usize;
		let bitvec_len = global::proofsize() * nonce_bits;
		size += bitvec_len / 8; // pow.nonces
		if bitvec_len % 8 != 0 {
			size += 1;
		}
		size
	}
}

/// A block as expressed in the MimbleWimble protocol. The reward is
/// non-explicit, assumed to be deducible from block height (similar to
/// bitcoin's schedule) and expressed as a global transaction fee (added v.H),
/// additive to the total of fees ever collected.
#[derive(Debug, Clone)]
pub struct Block {
	/// The header with metadata and commitments to the rest of the data
	pub header: BlockHeader,
	/// The body - inputs/outputs/kernels
	body: TransactionBody,
}

/// Implementation of Writeable for a block, defines how to write the block to a
/// binary writer. Differentiates between writing the block for the purpose of
/// full serialization and the one of just extracting a hash.
impl Writeable for Block {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.header.write(writer)?;

		if writer.serialization_mode() != ser::SerializationMode::Hash {
			self.body.write(writer)?;
		}
		Ok(())
	}
}

/// Implementation of Readable for a block, defines how to read a full block
/// from a binary stream.
impl Readable for Block {
	fn read(reader: &mut Reader) -> Result<Block, ser::Error> {
		let header = BlockHeader::read(reader)?;

		let body = TransactionBody::read(reader)?;

		// Now "lightweight" validation of the block.
		// Treat any validation issues as data corruption.
		// An example of this would be reading a block
		// that exceeded the allowed number of inputs.
		body.validate_read(true)
			.map_err(|_| ser::Error::CorruptedData)?;

		Ok(Block { header, body })
	}
}

/// Provides all information from a block that allows the calculation of total
/// Pedersen commitment.
impl Committed for Block {
	fn inputs_committed(&self) -> Vec<Commitment> {
		self.body.inputs_committed()
	}

	fn outputs_committed(&self) -> Vec<Commitment> {
		self.body.outputs_committed()
	}

	fn kernels_committed(&self) -> Vec<Commitment> {
		self.body.kernels_committed()
	}
}

/// Default properties for a block, everything zeroed out and empty vectors.
impl Default for Block {
	fn default() -> Block {
		Block {
			header: Default::default(),
			body: Default::default(),
		}
	}
}

impl Block {
	/// Builds a new block from the header of the previous block, a vector of
	/// transactions and the private key that will receive the reward. Checks
	/// that all transactions are valid and calculates the Merkle tree.
	///
	/// TODO - Move this somewhere where only tests will use it.
	/// *** Only used in tests. ***
	///
	pub fn new(
		prev: &BlockHeader,
		txs: Vec<Transaction>,
		difficulty: Difficulty,
		reward_output: (Output, TxKernel),
	) -> Result<Block, Error> {
		let mut block =
			Block::with_reward(prev, txs, reward_output.0, reward_output.1, difficulty)?;

		// Now set the pow on the header so block hashing works as expected.
		{
			let proof_size = global::proofsize();
			block.header.pow.proof = Proof::random(proof_size);
		}

		Ok(block)
	}

	/// Extract tx data from this block as a single aggregate tx.
	pub fn aggregate_transaction(
		&self,
		prev_kernel_offset: BlindingFactor,
	) -> Result<Option<Transaction>, Error> {
		let inputs = self.inputs().iter().cloned().collect();
		let outputs = self
			.outputs()
			.iter()
			.filter(|x| !x.features.contains(OutputFeatures::COINBASE_OUTPUT))
			.cloned()
			.collect();
		let kernels = self
			.kernels()
			.iter()
			.filter(|x| !x.features.contains(KernelFeatures::COINBASE_KERNEL))
			.cloned()
			.collect::<Vec<_>>();

		let tx = if kernels.is_empty() {
			None
		} else {
			let tx = Transaction::new(inputs, outputs, kernels)
				.with_offset(self.block_kernel_offset(prev_kernel_offset)?);
			Some(tx)
		};
		Ok(tx)
	}

	/// Hydrate a block from a compact block.
	/// Note: caller must validate the block themselves, we do not validate it
	/// here.
	pub fn hydrate_from(cb: CompactBlock, txs: Vec<Transaction>) -> Result<Block, Error> {
		trace!("block: hydrate_from: {}, {} txs", cb.hash(), txs.len(),);

		let header = cb.header.clone();

		let mut all_inputs = HashSet::new();
		let mut all_outputs = HashSet::new();
		let mut all_kernels = HashSet::new();

		// collect all the inputs, outputs and kernels from the txs
		for tx in txs {
			let tb: TransactionBody = tx.into();
			all_inputs.extend(tb.inputs);
			all_outputs.extend(tb.outputs);
			all_kernels.extend(tb.kernels);
		}

		// include the coinbase output(s) and kernel(s) from the compact_block
		{
			let body: CompactBlockBody = cb.into();
			all_outputs.extend(body.out_full);
			all_kernels.extend(body.kern_full);
		}

		// convert the sets to vecs
		let all_inputs = Vec::from_iter(all_inputs);
		let all_outputs = Vec::from_iter(all_outputs);
		let all_kernels = Vec::from_iter(all_kernels);

		// Initialize a tx body and sort everything.
		let body = TransactionBody::init(all_inputs, all_outputs, all_kernels, false)?;

		// Finally return the full block.
		// Note: we have not actually validated the block here,
		// caller must validate the block.
		Block { header, body }.cut_through()
	}

	/// Build a new empty block from a specified header
	pub fn with_header(header: BlockHeader) -> Block {
		Block {
			header,
			..Default::default()
		}
	}

	/// Builds a new block ready to mine from the header of the previous block,
	/// a vector of transactions and the reward information. Checks
	/// that all transactions are valid and calculates the Merkle tree.
	pub fn with_reward(
		prev: &BlockHeader,
		txs: Vec<Transaction>,
		reward_out: Output,
		reward_kern: TxKernel,
		difficulty: Difficulty,
	) -> Result<Block, Error> {
		// A block is just a big transaction, aggregate as such.
		let mut agg_tx = transaction::aggregate(txs)?;

		// Now add the reward output and reward kernel to the aggregate tx.
		// At this point the tx is technically invalid,
		// but the tx body is valid if we account for the reward (i.e. as a block).
		agg_tx = agg_tx.with_output(reward_out).with_kernel(reward_kern);

		// Now add the kernel offset of the previous block for a total
		let total_kernel_offset =
			committed::sum_kernel_offsets(vec![agg_tx.offset, prev.total_kernel_offset], vec![])?;

		let now = Utc::now().timestamp();
		let timestamp = DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(now, 0), Utc);

		// Now build the block with all the above information.
		// Note: We have not validated the block here.
		// Caller must validate the block as necessary.
		Block {
			header: BlockHeader {
				height: prev.height + 1,
				timestamp,
				previous: prev.hash(),
				total_kernel_offset,
				pow: ProofOfWork {
					total_difficulty: difficulty + prev.pow.total_difficulty,
					..Default::default()
				},
				..Default::default()
			},
			body: agg_tx.into(),
		}.cut_through()
	}

	/// Get inputs
	pub fn inputs(&self) -> &Vec<Input> {
		&self.body.inputs
	}

	/// Get inputs mutable
	pub fn inputs_mut(&mut self) -> &mut Vec<Input> {
		&mut self.body.inputs
	}

	/// Get outputs
	pub fn outputs(&self) -> &Vec<Output> {
		&self.body.outputs
	}

	/// Get outputs mutable
	pub fn outputs_mut(&mut self) -> &mut Vec<Output> {
		&mut self.body.outputs
	}

	/// Get kernels
	pub fn kernels(&self) -> &Vec<TxKernel> {
		&self.body.kernels
	}

	/// Get kernels mut
	pub fn kernels_mut(&mut self) -> &mut Vec<TxKernel> {
		&mut self.body.kernels
	}

	/// Blockhash, computed using only the POW
	pub fn hash(&self) -> Hash {
		self.header.hash()
	}

	/// Sum of all fees (inputs less outputs) in the block
	pub fn total_fees(&self) -> u64 {
		self.body
			.kernels
			.iter()
			.fold(0, |acc, ref x| acc.saturating_add(x.fee))
	}

	/// Matches any output with a potential spending input, eliminating them
	/// from the block. Provides a simple way to cut-through the block. The
	/// elimination is stable with respect to the order of inputs and outputs.
	/// Method consumes the block.
	///
	/// NOTE: exclude coinbase from cut-through process
	/// if a block contains a new coinbase output and
	/// is a transaction spending a previous coinbase
	/// we do not want to cut-through (all coinbase must be preserved)
	///
	pub fn cut_through(self) -> Result<Block, Error> {
		let mut inputs = self.inputs().clone();
		let mut outputs = self.outputs().clone();
		transaction::cut_through(&mut inputs, &mut outputs)?;

		let kernels = self.kernels().clone();

		// Initialize tx body and sort everything.
		let body = TransactionBody::init(inputs, outputs, kernels, false)?;

		Ok(Block {
			header: self.header,
			body,
		})
	}

	/// "Lightweight" validation that we can perform quickly during read/deserialization.
	/// Subset of full validation that skips expensive verification steps, specifically -
	/// * rangeproof verification (on the body)
	/// * kernel signature verification (on the body)
	/// * coinbase sum verification
	/// * kernel sum verification
	pub fn validate_read(&self) -> Result<(), Error> {
		self.body.validate_read(true)?;
		self.verify_kernel_lock_heights()?;
		Ok(())
	}

	fn block_kernel_offset(
		&self,
		prev_kernel_offset: BlindingFactor,
	) -> Result<BlindingFactor, Error> {
		let offset = if self.header.total_kernel_offset() == prev_kernel_offset {
			// special case when the sum hasn't changed (typically an empty block),
			// zero isn't a valid private key but it's a valid blinding factor
			BlindingFactor::zero()
		} else {
			committed::sum_kernel_offsets(
				vec![self.header.total_kernel_offset()],
				vec![prev_kernel_offset],
			)?
		};
		Ok(offset)
	}

	/// Validates all the elements in a block that can be checked without
	/// additional data. Includes commitment sums and kernels, Merkle
	/// trees, reward, etc.
	pub fn validate(
		&self,
		prev_kernel_offset: &BlindingFactor,
		verifier: Arc<RwLock<VerifierCache>>,
	) -> Result<Commitment, Error> {
		self.body.validate(true, verifier)?;

		self.verify_kernel_lock_heights()?;
		self.verify_coinbase()?;

		// take the kernel offset for this block (block offset minus previous) and
		// verify.body.outputs and kernel sums
		let (_utxo_sum, kernel_sum) = self.verify_kernel_sums(
			self.header.overage(),
			self.block_kernel_offset(*prev_kernel_offset)?,
		)?;

		Ok(kernel_sum)
	}

	/// Validate the coinbase.body.outputs generated by miners.
	/// Check the sum of coinbase-marked outputs match
	/// the sum of coinbase-marked kernels accounting for fees.
	pub fn verify_coinbase(&self) -> Result<(), Error> {
		let cb_outs = self
			.body
			.outputs
			.iter()
			.filter(|out| out.features.contains(OutputFeatures::COINBASE_OUTPUT))
			.collect::<Vec<&Output>>();

		let cb_kerns = self
			.body
			.kernels
			.iter()
			.filter(|kernel| kernel.features.contains(KernelFeatures::COINBASE_KERNEL))
			.collect::<Vec<&TxKernel>>();

		{
			let secp = static_secp_instance();
			let secp = secp.lock();
			let over_commit = secp.commit_value(reward(self.total_fees()))?;

			let out_adjust_sum = secp.commit_sum(
				cb_outs.iter().map(|x| x.commitment()).collect(),
				vec![over_commit],
			)?;

			let kerns_sum = secp.commit_sum(cb_kerns.iter().map(|x| x.excess).collect(), vec![])?;

			// Verify the kernel sum equals the output sum accounting for block fees.
			if kerns_sum != out_adjust_sum {
				return Err(Error::CoinbaseSumMismatch);
			}
		}

		Ok(())
	}

	fn verify_kernel_lock_heights(&self) -> Result<(), Error> {
		for k in &self.body.kernels {
			// check we have no kernels with lock_heights greater than current height
			// no tx can be included in a block earlier than its lock_height
			if k.lock_height > self.header.height {
				return Err(Error::KernelLockHeight(k.lock_height));
			}
		}
		Ok(())
	}
}
