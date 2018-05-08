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

use time;
use rand::{thread_rng, Rng};
use std::collections::HashSet;

use core::{Commitment, Committed, Input, KernelFeatures, Output, OutputFeatures, Proof, ShortId,
           Transaction, TxKernel};
use consensus;
use consensus::{exceeds_weight, reward, VerifySortOrder, REWARD};
use core::hash::{Hash, HashWriter, Hashed, ZERO_HASH};
use core::id::ShortIdentifiable;
use core::target::Difficulty;
use core::transaction;
use ser::{self, read_and_verify_sorted, Readable, Reader, Writeable, WriteableSorted, Writer};
use global;
use keychain;
use keychain::BlindingFactor;
use util::LOGGER;
use util::{secp, static_secp_instance};

/// Errors thrown by Block validation
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
	/// The sum of output minus input commitments does not
	/// match the sum of kernel commitments
	KernelSumMismatch,
	/// Same as above but for the coinbase part of a block, including reward
	CoinbaseSumMismatch,
	/// Too many inputs, outputs or kernels in the block
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
	/// Coinbase has not yet matured and cannot be spent (1,000 blocks)
	ImmatureCoinbase {
		/// The height of the block containing the input spending the coinbase
		/// output
		height: u64,
		/// The lock_height needed to be reached for the coinbase output to
		/// mature
		lock_height: u64,
	},
	/// Underlying Merkle proof error
	MerkleProof,
	/// Other unspecified error condition
	Other(String),
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

/// Block header, fairly standard compared to other blockchains.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockHeader {
	/// Version of the block
	pub version: u16,
	/// Height of this block since the genesis block (height 0)
	pub height: u64,
	/// Hash of the block previous to this in the chain.
	pub previous: Hash,
	/// Timestamp at which the block was built.
	pub timestamp: time::Tm,
	/// Total accumulated difficulty since genesis block
	pub total_difficulty: Difficulty,
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
	/// Nonce increment used to mine this block.
	pub nonce: u64,
	/// Proof of work data.
	pub pow: Proof,
}

impl Default for BlockHeader {
	fn default() -> BlockHeader {
		let proof_size = global::proofsize();
		BlockHeader {
			version: 1,
			height: 0,
			previous: ZERO_HASH,
			timestamp: time::at_utc(time::Timespec { sec: 0, nsec: 0 }),
			total_difficulty: Difficulty::one(),
			output_root: ZERO_HASH,
			range_proof_root: ZERO_HASH,
			kernel_root: ZERO_HASH,
			total_kernel_offset: BlindingFactor::zero(),
			nonce: 0,
			pow: Proof::zero(proof_size),
		}
	}
}

/// Serialization of a block header
impl Writeable for BlockHeader {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		if writer.serialization_mode() != ser::SerializationMode::Hash {
			self.write_pre_pow(writer)?;
		}

		self.pow.write(writer)?;
		Ok(())
	}
}

/// Deserialization of a block header
impl Readable for BlockHeader {
	fn read(reader: &mut Reader) -> Result<BlockHeader, ser::Error> {
		let (version, height) = ser_multiread!(reader, read_u16, read_u64);
		let previous = Hash::read(reader)?;
		let timestamp = reader.read_i64()?;
		let total_difficulty = Difficulty::read(reader)?;
		let output_root = Hash::read(reader)?;
		let rproof_root = Hash::read(reader)?;
		let kernel_root = Hash::read(reader)?;
		let total_kernel_offset = BlindingFactor::read(reader)?;
		let nonce = reader.read_u64()?;
		let pow = Proof::read(reader)?;

		if timestamp > (1 << 55) || timestamp < -(1 << 55) {
			return Err(ser::Error::CorruptedData);
		}

		Ok(BlockHeader {
			version: version,
			height: height,
			previous: previous,
			timestamp: time::at_utc(time::Timespec {
				sec: timestamp,
				nsec: 0,
			}),
			total_difficulty: total_difficulty,
			output_root: output_root,
			range_proof_root: rproof_root,
			kernel_root: kernel_root,
			total_kernel_offset: total_kernel_offset,
			nonce: nonce,
			pow: pow,
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
			[write_fixed_bytes, &self.previous],
			[write_i64, self.timestamp.to_timespec().sec],
			[write_u64, self.total_difficulty.into_num()],
			[write_fixed_bytes, &self.output_root],
			[write_fixed_bytes, &self.range_proof_root],
			[write_fixed_bytes, &self.kernel_root],
			[write_fixed_bytes, &self.total_kernel_offset],
			[write_u64, self.nonce]
		);
		Ok(())
	}
	///
	/// Returns the pre-pow hash, as the post-pow hash
	/// should just be the hash of the POW
	pub fn pre_pow_hash(&self) -> Hash {
		let mut hasher = HashWriter::default();
		self.write_pre_pow(&mut hasher).unwrap();
		let mut ret = [0; 32];
		hasher.finalize(&mut ret);
		Hash(ret)
	}
}

/// Compact representation of a full block.
/// Each input/output/kernel is represented as a short_id.
/// A node is reasonably likely to have already seen all tx data (tx broadcast before block)
/// and can go request missing tx data from peers if necessary to hydrate a compact block
/// into a full block.
#[derive(Debug, Clone)]
pub struct CompactBlock {
	/// The header with metadata and commitments to the rest of the data
	pub header: BlockHeader,
	/// Nonce for connection specific short_ids
	pub nonce: u64,
	/// List of full outputs - specifically the coinbase output(s)
	pub out_full: Vec<Output>,
	/// List of full kernels - specifically the coinbase kernel(s)
	pub kern_full: Vec<TxKernel>,
	/// List of transaction kernels, excluding those in the full list
	/// (short_ids)
	pub kern_ids: Vec<ShortId>,
}

/// Implementation of Writeable for a compact block, defines how to write the block to a
/// binary writer. Differentiates between writing the block for the purpose of
/// full serialization and the one of just extracting a hash.
impl Writeable for CompactBlock {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		try!(self.header.write(writer));

		if writer.serialization_mode() != ser::SerializationMode::Hash {
			try!(writer.write_u64(self.nonce));

			ser_multiwrite!(
				writer,
				[write_u64, self.out_full.len() as u64],
				[write_u64, self.kern_full.len() as u64],
				[write_u64, self.kern_ids.len() as u64]
			);

			let mut out_full = self.out_full.clone();
			let mut kern_full = self.kern_full.clone();
			let mut kern_ids = self.kern_ids.clone();

			// Consensus rule that everything is sorted in lexicographical order on the
			// wire.
			try!(out_full.write_sorted(writer));
			try!(kern_full.write_sorted(writer));
			try!(kern_ids.write_sorted(writer));
		}
		Ok(())
	}
}

/// Implementation of Readable for a compact block, defines how to read a compact block
/// from a binary stream.
impl Readable for CompactBlock {
	fn read(reader: &mut Reader) -> Result<CompactBlock, ser::Error> {
		let header = try!(BlockHeader::read(reader));

		let (nonce, out_full_len, kern_full_len, kern_id_len) =
			ser_multiread!(reader, read_u64, read_u64, read_u64, read_u64);

		let out_full = read_and_verify_sorted(reader, out_full_len as u64)?;
		let kern_full = read_and_verify_sorted(reader, kern_full_len as u64)?;
		let kern_ids = read_and_verify_sorted(reader, kern_id_len)?;

		Ok(CompactBlock {
			header,
			nonce,
			out_full,
			kern_full,
			kern_ids,
		})
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
	/// List of transaction inputs
	pub inputs: Vec<Input>,
	/// List of transaction outputs
	pub outputs: Vec<Output>,
	/// List of kernels with associated proofs (note these are offset from
	/// tx_kernels)
	pub kernels: Vec<TxKernel>,
}

/// Implementation of Writeable for a block, defines how to write the block to a
/// binary writer. Differentiates between writing the block for the purpose of
/// full serialization and the one of just extracting a hash.
impl Writeable for Block {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		try!(self.header.write(writer));

		if writer.serialization_mode() != ser::SerializationMode::Hash {
			ser_multiwrite!(
				writer,
				[write_u64, self.inputs.len() as u64],
				[write_u64, self.outputs.len() as u64],
				[write_u64, self.kernels.len() as u64]
			);

			let mut inputs = self.inputs.clone();
			let mut outputs = self.outputs.clone();
			let mut kernels = self.kernels.clone();

			// Consensus rule that everything is sorted in lexicographical order on the
			// wire.
			try!(inputs.write_sorted(writer));
			try!(outputs.write_sorted(writer));
			try!(kernels.write_sorted(writer));
		}
		Ok(())
	}
}

/// Implementation of Readable for a block, defines how to read a full block
/// from a binary stream.
impl Readable for Block {
	fn read(reader: &mut Reader) -> Result<Block, ser::Error> {
		let header = try!(BlockHeader::read(reader));

		let (input_len, output_len, kernel_len) =
			ser_multiread!(reader, read_u64, read_u64, read_u64);

		let inputs = read_and_verify_sorted(reader, input_len)?;
		let outputs = read_and_verify_sorted(reader, output_len)?;
		let kernels = read_and_verify_sorted(reader, kernel_len)?;

		Ok(Block {
			header: header,
			inputs: inputs,
			outputs: outputs,
			kernels: kernels,
			..Default::default()
		})
	}
}

/// Provides all information from a block that allows the calculation of total
/// Pedersen commitment.
impl Committed for Block {
	fn inputs_committed(&self) -> Vec<Commitment> {
		self.inputs.iter().map(|x| x.commitment()).collect()
	}

	fn outputs_committed(&self) -> Vec<Commitment> {
		self.outputs.iter().map(|x| x.commitment()).collect()
	}

	fn kernels_committed(&self) -> Vec<Commitment> {
		self.kernels.iter().map(|x| x.excess()).collect()
	}
}

/// Default properties for a block, everything zeroed out and empty vectors.
impl Default for Block {
	fn default() -> Block {
		Block {
			header: Default::default(),
			inputs: vec![],
			outputs: vec![],
			kernels: vec![],
		}
	}
}

impl Block {
	/// Builds a new block from the header of the previous block, a vector of
	/// transactions and the private key that will receive the reward. Checks
	/// that all transactions are valid and calculates the Merkle tree.
	///
	/// Only used in tests (to be confirmed, may be wrong here).
	///
	pub fn new(
		prev: &BlockHeader,
		txs: Vec<&Transaction>,
		difficulty: Difficulty,
		reward_output: (Output, TxKernel),
	) -> Result<Block, Error> {
		let block = Block::with_reward(prev, txs, reward_output.0, reward_output.1, difficulty)?;
		Ok(block)
	}

	/// Hydrate a block from a compact block.
	/// Note: caller must validate the block themselves, we do not validate it here.
	pub fn hydrate_from(cb: CompactBlock, txs: Vec<Transaction>) -> Block {
		trace!(
			LOGGER,
			"block: hydrate_from: {}, {} txs",
			cb.hash(),
			txs.len(),
		);

		let mut all_inputs = HashSet::new();
		let mut all_outputs = HashSet::new();
		let mut all_kernels = HashSet::new();

		// collect all the inputs, outputs and kernels from the txs
		for tx in txs {
			all_inputs.extend(tx.inputs);
			all_outputs.extend(tx.outputs);
			all_kernels.extend(tx.kernels);
		}

		// include the coinbase output(s) and kernel(s) from the compact_block
		all_outputs.extend(cb.out_full);
		all_kernels.extend(cb.kern_full);

		// convert the sets to vecs
		let mut all_inputs = all_inputs.iter().cloned().collect::<Vec<_>>();
		let mut all_outputs = all_outputs.iter().cloned().collect::<Vec<_>>();
		let mut all_kernels = all_kernels.iter().cloned().collect::<Vec<_>>();

		// sort them all lexicographically
		all_inputs.sort();
		all_outputs.sort();
		all_kernels.sort();

		// finally return the full block
		// Note: we have not actually validated the block here
		// leave it to the caller to actually validate the block
		Block {
			header: cb.header,
			inputs: all_inputs,
			outputs: all_outputs,
			kernels: all_kernels,
		}.cut_through()
	}

	/// Generate the compact block representation.
	pub fn as_compact_block(&self) -> CompactBlock {
		let header = self.header.clone();
		let nonce = thread_rng().next_u64();

		let mut out_full = self.outputs
			.iter()
			.filter(|x| x.features.contains(OutputFeatures::COINBASE_OUTPUT))
			.cloned()
			.collect::<Vec<_>>();

		let mut kern_full = vec![];
		let mut kern_ids = vec![];

		for k in &self.kernels {
			if k.features.contains(KernelFeatures::COINBASE_KERNEL) {
				kern_full.push(k.clone());
			} else {
				kern_ids.push(k.short_id(&header.hash(), nonce));
			}
		}

		// sort all the lists
		out_full.sort();
		kern_full.sort();
		kern_ids.sort();

		CompactBlock {
			header,
			nonce,
			out_full,
			kern_full,
			kern_ids,
		}
	}

	/// Builds a new block ready to mine from the header of the previous block,
	/// a vector of transactions and the reward information. Checks
	/// that all transactions are valid and calculates the Merkle tree.
	pub fn with_reward(
		prev: &BlockHeader,
		txs: Vec<&Transaction>,
		reward_out: Output,
		reward_kern: TxKernel,
		difficulty: Difficulty,
	) -> Result<Block, Error> {
		let mut kernels = vec![];
		let mut inputs = vec![];
		let mut outputs = vec![];

		// we will sum these together at the end
		// to give us the overall offset for the block
		let mut kernel_offsets = vec![];

		// iterate over the all the txs
		// build the kernel for each
		// and collect all the kernels, inputs and outputs
		// to build the block (which we can sort of think of as one big tx?)
		for tx in txs {
			// validate each transaction and gather their kernels
			// tx has an offset k2 where k = k1 + k2
			// and the tx is signed using k1
			// the kernel excess is k1G
			// we will sum all the offsets later and store the total offset
			// on the block_header
			tx.validate()?;

			// we will summ these later to give a single aggregate offset
			kernel_offsets.push(tx.offset);

			// add all tx inputs/outputs/kernels to the block
			kernels.extend(tx.kernels.iter().cloned());
			inputs.extend(tx.inputs.iter().cloned());
			outputs.extend(tx.outputs.iter().cloned());
		}

		// include the reward kernel and output
		kernels.push(reward_kern);
		outputs.push(reward_out);

		// now sort everything so the block is built deterministically
		inputs.sort();
		outputs.sort();
		kernels.sort();

		// now sum the kernel_offsets up to give us
		// an aggregate offset for the entire block
		let total_kernel_offset = {
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			let mut keys = kernel_offsets
				.iter()
				.cloned()
				.filter(|x| *x != BlindingFactor::zero())
				.filter_map(|x| x.secret_key(&secp).ok())
				.collect::<Vec<_>>();
			if prev.total_kernel_offset != BlindingFactor::zero() {
				keys.push(prev.total_kernel_offset.secret_key(&secp)?);
			}

			if keys.is_empty() {
				BlindingFactor::zero()
			} else {
				let sum = secp.blind_sum(keys, vec![])?;
				BlindingFactor::from_secret_key(sum)
			}
		};

		Ok(Block {
			header: BlockHeader {
				height: prev.height + 1,
				timestamp: time::Tm {
					tm_nsec: 0,
					..time::now_utc()
				},
				previous: prev.hash(),
				total_difficulty: difficulty + prev.total_difficulty.clone(),
				total_kernel_offset: total_kernel_offset,
				..Default::default()
			},
			inputs: inputs,
			outputs: outputs,
			kernels: kernels,
		}.cut_through())
	}

	/// Blockhash, computed using only the POW
	pub fn hash(&self) -> Hash {
		self.header.hash()
	}

	/// Sum of all fees (inputs less outputs) in the block
	pub fn total_fees(&self) -> u64 {
		self.kernels.iter().map(|p| p.fee).sum()
	}

	/// Matches any output with a potential spending input, eliminating them
	/// from the block. Provides a simple way to cut-through the block. The
	/// elimination is stable with respect to the order of inputs and outputs.
	///
	/// NOTE: exclude coinbase from cut-through process
	/// if a block contains a new coinbase output and
	/// is a transaction spending a previous coinbase
	/// we do not want to cut-through (all coinbase must be preserved)
	///
	pub fn cut_through(&self) -> Block {
		let in_set = self.inputs
			.iter()
			.map(|inp| inp.commitment())
			.collect::<HashSet<_>>();

		let out_set = self.outputs
			.iter()
			.filter(|out| !out.features.contains(OutputFeatures::COINBASE_OUTPUT))
			.map(|out| out.commitment())
			.collect::<HashSet<_>>();

		let to_cut_through = in_set.intersection(&out_set).collect::<HashSet<_>>();

		let new_inputs = self.inputs
			.iter()
			.filter(|inp| !to_cut_through.contains(&inp.commitment()))
			.cloned()
			.collect::<Vec<_>>();

		let new_outputs = self.outputs
			.iter()
			.filter(|out| !to_cut_through.contains(&out.commitment()))
			.cloned()
			.collect::<Vec<_>>();

		Block {
			header: BlockHeader {
				pow: self.header.pow.clone(),
				total_difficulty: self.header.total_difficulty.clone(),
				..self.header
			},
			inputs: new_inputs,
			outputs: new_outputs,
			kernels: self.kernels.clone(),
		}
	}

	/// Validates all the elements in a block that can be checked without
	/// additional data. Includes commitment sums and kernels, Merkle
	/// trees, reward, etc.
	pub fn validate(
		&self,
		prev_output_sum: &Commitment,
		prev_kernel_sum: &Commitment,
	) -> Result<((Commitment, Commitment)), Error> {
		self.verify_weight()?;
		self.verify_sorted()?;
		self.verify_coinbase()?;
		self.verify_inputs()?;
		self.verify_kernel_lock_heights()?;
		let (new_output_sum, new_kernel_sum) = self.verify_sums(prev_output_sum, prev_kernel_sum)?;

		Ok((new_output_sum, new_kernel_sum))
	}

	fn verify_weight(&self) -> Result<(), Error> {
		if exceeds_weight(self.inputs.len(), self.outputs.len(), self.kernels.len()) {
			return Err(Error::WeightExceeded);
		}
		Ok(())
	}

	fn verify_sorted(&self) -> Result<(), Error> {
		self.inputs.verify_sort_order()?;
		self.outputs.verify_sort_order()?;
		self.kernels.verify_sort_order()?;
		Ok(())
	}

	/// We can verify the Merkle proof (for coinbase inputs) here in isolation.
	/// But we cannot check the following as we need data from the index and the PMMR.
	/// So we must be sure to check these at the appropriate point during block validation.
	///   * node is in the correct pos in the PMMR
	///   * block is the correct one (based on output_root from block_header via the index)
	fn verify_inputs(&self) -> Result<(), Error> {
		let coinbase_inputs = self.inputs
			.iter()
			.filter(|x| x.features.contains(OutputFeatures::COINBASE_OUTPUT));

		for input in coinbase_inputs {
			let merkle_proof = input.merkle_proof();
			if !merkle_proof.verify() {
				return Err(Error::MerkleProof);
			}
		}

		Ok(())
	}

	fn verify_kernel_lock_heights(&self) -> Result<(), Error> {
		for k in &self.kernels {
			// check we have no kernels with lock_heights greater than current height
			// no tx can be included in a block earlier than its lock_height
			if k.lock_height > self.header.height {
				return Err(Error::KernelLockHeight(k.lock_height));
			}
		}
		Ok(())
	}

	/// Verify sums
	pub fn verify_sums(
		&self,
		prev_output_sum: &Commitment,
		prev_kernel_sum: &Commitment,
	) -> Result<((Commitment, Commitment)), Error> {
		// Verify the output rangeproofs.
		// Note: this is expensive.
		for x in &self.outputs {
			x.verify_proof()?;
		}

		// Verify the kernel signatures.
		// Note: this is expensive.
		for x in &self.kernels {
			x.verify()?;
		}

		// Sum all input|output|overage commitments.
		let overage = (REWARD as i64).checked_neg().unwrap_or(0);
		let io_sum = self.sum_commitments(overage, Some(prev_output_sum))?;

		let offset = {
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			let key = self.header.total_kernel_offset.secret_key(&secp)?;
			secp.commit(0, key)?
		};

		// Sum the kernel excesses accounting for the kernel offset.
		let (kernel_sum, kernel_sum_plus_offset) =
			self.sum_kernel_excesses(&offset, Some(prev_kernel_sum))?;

		if io_sum != kernel_sum_plus_offset {
			return Err(Error::KernelSumMismatch);
		}

		Ok((io_sum, kernel_sum))
	}

	/// Validate the coinbase outputs generated by miners. Entails 2 main checks:
	///
	/// * That the sum of all coinbase-marked outputs equal the supply.
	/// * That the sum of blinding factors for all coinbase-marked outputs match
	///   the coinbase-marked kernels.
	pub fn verify_coinbase(&self) -> Result<(), Error> {
		let cb_outs = self.outputs
			.iter()
			.filter(|out| out.features.contains(OutputFeatures::COINBASE_OUTPUT))
			.cloned()
			.collect::<Vec<Output>>();

		let cb_kerns = self.kernels
			.iter()
			.filter(|kernel| kernel.features.contains(KernelFeatures::COINBASE_KERNEL))
			.cloned()
			.collect::<Vec<TxKernel>>();

		let over_commit;
		let out_adjust_sum;
		let kerns_sum;
		{
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			over_commit = secp.commit_value(reward(self.total_fees()))?;
			out_adjust_sum = secp.commit_sum(
				cb_outs.iter().map(|x| x.commitment()).collect(),
				vec![over_commit],
			)?;
			kerns_sum = secp.commit_sum(cb_kerns.iter().map(|x| x.excess).collect(), vec![])?;
		}

		if kerns_sum != out_adjust_sum {
			return Err(Error::CoinbaseSumMismatch);
		}
		Ok(())
	}
}
