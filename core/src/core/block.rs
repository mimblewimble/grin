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

//! Blocks and blockheaders

use time;
use util;
use util::{secp, static_secp_instance};
use std::collections::HashSet;

use core::{
	Committed,
	Input,
	Output,
	OutputIdentifier,
	ShortId,
	SwitchCommitHash,
	Proof,
	TxKernel,
	Transaction,
	COINBASE_KERNEL,
	COINBASE_OUTPUT
};
use consensus;
use consensus::{exceeds_weight, reward, REWARD, VerifySortOrder};
use core::hash::{Hash, Hashed, ZERO_HASH};
use core::id::ShortIdentifiable;
use core::target::Difficulty;
use core::transaction;
use ser::{self, Readable, Reader, Writeable, Writer, WriteableSorted, read_and_verify_sorted};
use util::kernel_sig_msg;
use util::LOGGER;
use global;
use keychain;

/// Errors thrown by Block validation
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
	/// The sum of output minus input commitments does not match the sum of
	/// kernel commitments
	KernelSumMismatch,
	/// Same as above but for the coinbase part of a block, including reward
	CoinbaseSumMismatch,
	/// Kernel fee can't be odd, due to half fee burning
	OddKernelFee,
	/// Too many inputs, outputs or kernels in the block
	WeightExceeded,
	/// Kernel not valid due to lock_height exceeding block header height
	KernelLockHeight(u64),
	/// Underlying tx related error
	Transaction(transaction::Error),
	/// Underlying Secp256k1 error (signature validation or invalid public key typically)
	Secp(secp::Error),
	/// Underlying keychain related error
	Keychain(keychain::Error),
	/// Underlying consensus error (sort order currently)
	Consensus(consensus::Error),
	/// Coinbase has not yet matured and cannot be spent (1,000 blocks)
	ImmatureCoinbase {
		/// The height of the block containing the input spending the coinbase output
		height: u64,
		/// The lock_height needed to be reached for the coinbase output to mature
		lock_height: u64,
	},
	/// Limit on number of coinbase outputs in a valid block.
	CoinbaseOutputCountExceeded,
	/// Limit on number of coinbase kernels in a valid block.
	CoinbaseKernelCountExceeded,
	/// Other unspecified error condition
	Other(String)
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
	/// Merklish root of all the commitments in the UTXO set
	pub utxo_root: Hash,
	/// Merklish root of all range proofs in the UTXO set
	pub range_proof_root: Hash,
	/// Merklish root of all transaction kernels in the UTXO set
	pub kernel_root: Hash,
	/// Nonce increment used to mine this block.
	pub nonce: u64,
	/// Proof of work data.
	pub pow: Proof,
	/// Difficulty used to mine the block.
	pub difficulty: Difficulty,
	/// Total accumulated difficulty since genesis block
	pub total_difficulty: Difficulty,
}

impl Default for BlockHeader {
	fn default() -> BlockHeader {
		let proof_size = global::proofsize();
		BlockHeader {
			version: 1,
			height: 0,
			previous: ZERO_HASH,
			timestamp: time::at_utc(time::Timespec { sec: 0, nsec: 0 }),
			difficulty: Difficulty::one(),
			total_difficulty: Difficulty::one(),
			utxo_root: ZERO_HASH,
			range_proof_root: ZERO_HASH,
			kernel_root: ZERO_HASH,
			nonce: 0,
			pow: Proof::zero(proof_size),
		}
	}
}

/// Serialization of a block header
impl Writeable for BlockHeader {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u16, self.version],
			[write_u64, self.height],
			[write_fixed_bytes, &self.previous],
			[write_i64, self.timestamp.to_timespec().sec],
			[write_fixed_bytes, &self.utxo_root],
			[write_fixed_bytes, &self.range_proof_root],
			[write_fixed_bytes, &self.kernel_root]
		);

		try!(writer.write_u64(self.nonce));
		try!(self.difficulty.write(writer));
		try!(self.total_difficulty.write(writer));

		if writer.serialization_mode() != ser::SerializationMode::Hash {
			try!(self.pow.write(writer));
		}
		Ok(())
	}
}

/// Deserialization of a block header
impl Readable for BlockHeader {
	fn read(reader: &mut Reader) -> Result<BlockHeader, ser::Error> {
		let (version, height) = ser_multiread!(reader, read_u16, read_u64);
		let previous = Hash::read(reader)?;
		let timestamp = reader.read_i64()?;
		let utxo_root = Hash::read(reader)?;
		let rproof_root = Hash::read(reader)?;
		let kernel_root = Hash::read(reader)?;
		let nonce = reader.read_u64()?;
		let difficulty = Difficulty::read(reader)?;
		let total_difficulty = Difficulty::read(reader)?;
		let pow = Proof::read(reader)?;

		Ok(BlockHeader {
			version: version,
			height: height,
			previous: previous,
			timestamp: time::at_utc(time::Timespec {
				sec: timestamp,
				nsec: 0,
			}),
			utxo_root: utxo_root,
			range_proof_root: rproof_root,
			kernel_root: kernel_root,
			pow: pow,
			nonce: nonce,
			difficulty: difficulty,
			total_difficulty: total_difficulty,
		})
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
	/// List of full outputs - specifically the coinbase output(s)
	pub out_full: Vec<Output>,
	/// List of full kernels - specifically the coinbase kernel(s)
	pub kern_full: Vec<TxKernel>,
	/// List of transaction inputs (short_ids)
	pub in_ids: Vec<ShortId>,
	/// List of transaction outputs, excluding those in the full list (short_ids)
	pub out_ids: Vec<ShortId>,
	/// List of transaction kernels, excluding those in the full list (short_ids)
	pub kern_ids: Vec<ShortId>,
}

/// Implementation of Writeable for a compact block, defines how to write the block to a
/// binary writer. Differentiates between writing the block for the purpose of
/// full serialization and the one of just extracting a hash.
impl Writeable for CompactBlock {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		try!(self.header.write(writer));

		if writer.serialization_mode() != ser::SerializationMode::Hash {
			// TODO - make these constants and put them somewhere reusable?
			assert!(self.out_full.len() < 16);
			assert!(self.kern_full.len() < 16);

			ser_multiwrite!(
				writer,
				[write_u8, self.out_full.len() as u8],
				[write_u8, self.kern_full.len() as u8],
				[write_u64, self.in_ids.len() as u64],
				[write_u64, self.out_ids.len() as u64],
				[write_u64, self.kern_ids.len() as u64]
			);

			let mut out_full = self.out_full.clone();
			let mut kern_full = self.kern_full.clone();

			let mut in_ids = self.in_ids.clone();
			let mut out_ids = self.out_ids.clone();
			let mut kern_ids = self.kern_ids.clone();

			// Consensus rule that everything is sorted in lexicographical order on the wire.
			try!(out_full.write_sorted(writer));
			try!(kern_full.write_sorted(writer));
			try!(in_ids.write_sorted(writer));
			try!(out_ids.write_sorted(writer));
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

		let (out_full_len, kern_full_len, in_id_len, out_id_len, kern_id_len) =
			ser_multiread!(reader, read_u8, read_u8, read_u64, read_u64, read_u64);

		let out_full = read_and_verify_sorted(reader, out_full_len as u64)?;
		let kern_full = read_and_verify_sorted(reader, kern_full_len as u64)?;
		let in_ids = read_and_verify_sorted(reader, in_id_len)?;
		let out_ids = read_and_verify_sorted(reader, out_id_len)?;
		let kern_ids = read_and_verify_sorted(reader, kern_id_len)?;

		Ok(CompactBlock {
			header,
			out_full,
			kern_full,
			in_ids,
			out_ids,
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
	/// List of transaction kernels and associated proofs
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

			// Consensus rule that everything is sorted in lexicographical order on the wire.
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
	fn inputs_committed(&self) -> &Vec<Input> {
		&self.inputs
	}
	fn outputs_committed(&self) -> &Vec<Output> {
		&self.outputs
	}
	fn overage(&self) -> i64 {
		((self.total_fees() / 2) as i64) - (REWARD as i64)
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
		keychain: &keychain::Keychain,
		key_id: &keychain::Identifier,
		difficulty: Difficulty,
	) -> Result<Block, Error> {
		let fees = txs.iter().map(|tx| tx.fee).sum();
		let (reward_out, reward_proof) = Block::reward_output(
			keychain,
			key_id,
			fees,
			prev.height + 1,
		)?;
		let block = Block::with_reward(prev, txs, reward_out, reward_proof, difficulty)?;
		Ok(block)
	}

	/// Generate the compact block representation.
	pub fn as_compact_block(&self) -> CompactBlock {
		let header = self.header.clone();
		let block_hash = self.hash();

		let mut out_full = self.outputs
			.iter()
			.filter(|x| x.features.contains(COINBASE_OUTPUT))
			.cloned()
			.collect::<Vec<_>>();
		let mut kern_full = self.kernels
			.iter()
			.filter(|x| x.features.contains(COINBASE_KERNEL))
			.cloned()
			.collect::<Vec<_>>();

		let mut in_ids = self.inputs
			.iter()
			.map(|x| x.short_id(&block_hash))
			.collect::<Vec<_>>();
		let mut out_ids = self.outputs
			.iter()
			.filter(|x| !x.features.contains(COINBASE_OUTPUT))
			.map(|x| x.short_id(&block_hash))
			.collect::<Vec<_>>();
		let mut kern_ids = self.kernels
			.iter()
			.filter(|x| !x.features.contains(COINBASE_KERNEL))
			.map(|x| x.short_id(&block_hash))
			.collect::<Vec<_>>();

		// sort all the lists
		out_full.sort();
		kern_full.sort();
		in_ids.sort();
		out_ids.sort();
		kern_ids.sort();

		CompactBlock {
			header,
			out_full,
			kern_full,
			in_ids,
			out_ids,
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

		// iterate over the all the txs
		// build the kernel for each
		// and collect all the kernels, inputs and outputs
		// to build the block (which we can sort of think of as one big tx?)
		for tx in txs {
			// validate each transaction and gather their kernels
			let excess = tx.validate()?;
			let kernel = tx.build_kernel(excess);
			kernels.push(kernel);

			for input in tx.inputs.clone() {
				inputs.push(input);
			}

			for output in tx.outputs.clone() {
				outputs.push(output);
			}
		}

		// also include the reward kernel and output
		kernels.push(reward_kern);
		outputs.push(reward_out);

		// now sort everything so the block is built deterministically
		inputs.sort();
		outputs.sort();
		kernels.sort();

		// calculate the overall Merkle tree and fees (todo?)
		Ok(
			Block {
				header: BlockHeader {
					height: prev.height + 1,
					timestamp: time::Tm {
						tm_nsec: 0,
						..time::now_utc()
					},
					previous: prev.hash(),
					total_difficulty: difficulty +
						prev.total_difficulty.clone(),
					..Default::default()
				},
				inputs: inputs,
				outputs: outputs,
				kernels: kernels,
			}.cut_through(),
		)
	}

	/// Blockhash, computed using only the header
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
			.filter(|out| !out.features.contains(COINBASE_OUTPUT))
			.map(|out| out.commitment())
			.collect::<HashSet<_>>();

		let to_cut_through = in_set.intersection(&out_set).collect::<HashSet<_>>();

		let new_inputs = self.inputs
			.iter()
			.filter(|inp| !to_cut_through.contains(&inp.commitment()))
			.map(|&inp| inp)
			.collect::<Vec<_>>();

		let new_outputs = self.outputs
			.iter()
			.filter(|out| !to_cut_through.contains(&out.commitment()))
			.map(|&out| out)
			.collect::<Vec<_>>();

		Block {
			header: BlockHeader {
				pow: self.header.pow.clone(),
				difficulty: self.header.difficulty.clone(),
				total_difficulty: self.header.total_difficulty.clone(),
				..self.header
			},
			inputs: new_inputs,
			outputs: new_outputs,
			kernels: self.kernels.clone(),
		}
	}

	/// Merges the 2 blocks, essentially appending the inputs, outputs and
	/// kernels.
	/// Also performs a compaction on the result.
	pub fn merge(&self, other: Block) -> Block {
		let mut all_inputs = self.inputs.clone();
		all_inputs.append(&mut other.inputs.clone());

		let mut all_outputs = self.outputs.clone();
		all_outputs.append(&mut other.outputs.clone());

		let mut all_kernels = self.kernels.clone();
		all_kernels.append(&mut other.kernels.clone());

		Block {
			// compact will fix the merkle tree
			header: BlockHeader {
				pow: self.header.pow.clone(),
				difficulty: self.header.difficulty.clone(),
				total_difficulty: self.header.total_difficulty.clone(),
				..self.header
			},
			inputs: all_inputs,
			outputs: all_outputs,
			kernels: all_kernels,
		}.cut_through()
	}

	/// Validates all the elements in a block that can be checked without
	/// additional data. Includes commitment sums and kernels, Merkle
	/// trees, reward, etc.
	///
	/// TODO - performs various verification steps - discuss renaming this to "verify"
	/// as all the steps within are verify steps.
	///
	pub fn validate(&self) -> Result<(), Error> {
		self.verify_weight()?;
		self.verify_sorted()?;
		self.verify_coinbase()?;
		self.verify_kernels()?;
		Ok(())
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

	/// Verifies the sum of input/output commitments match the sum in kernels
	/// and that all kernel signatures are valid.
	fn verify_kernels(&self) -> Result<(), Error> {
		for k in &self.kernels {
			if k.fee & 1 != 0 {
				return Err(Error::OddKernelFee);
			}

			// check we have no kernels with lock_heights greater than current height
			// no tx can be included in a block earlier than its lock_height
			if k.lock_height > self.header.height {
				return Err(Error::KernelLockHeight(k.lock_height));
			}
		}

		// sum all inputs and outs commitments
		let io_sum = self.sum_commitments()?;

		// sum all kernels commitments
		let proof_commits = map_vec!(self.kernels, |proof| proof.excess);

		let proof_sum = {
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			secp.commit_sum(proof_commits, vec![])?
		};

		// both should be the same
		if proof_sum != io_sum {
			return Err(Error::KernelSumMismatch);
		}

		// verify all signatures with the commitment as pk
		for proof in &self.kernels {
			proof.verify()?;
		}

		Ok(())
	}

	/// Validate the coinbase outputs generated by miners. Entails 3 main checks:
	///
	/// * That the block does not exceed the number of permitted coinbase outputs or kernels.
	/// * That the sum of all coinbase-marked outputs equal the supply.
	/// * That the sum of blinding factors for all coinbase-marked outputs match
	///   the coinbase-marked kernels.
	fn verify_coinbase(&self) -> Result<(), Error> {
		let cb_outs = self.outputs
			.iter()
			.filter(|out| out.features.contains(COINBASE_OUTPUT))
			.cloned()
			.collect::<Vec<Output>>();

		let cb_kerns = self.kernels
			.iter()
			.filter(|kernel| kernel.features.contains(COINBASE_KERNEL))
			.cloned()
			.collect::<Vec<TxKernel>>();

		// First check that we do not have too many coinbase outputs in the block.
		if cb_outs.len() as u64 > consensus::MAX_BLOCK_COINBASE_OUTPUTS {
			return Err(Error::CoinbaseOutputCountExceeded);
		}

		// And that we do not have too many coinbase kernels in the block.
		if cb_kerns.len() as u64 > consensus::MAX_BLOCK_COINBASE_KERNELS {
			return Err(Error::CoinbaseKernelCountExceeded);
		}

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
			kerns_sum = secp.commit_sum(
				cb_kerns.iter().map(|x| x.excess).collect(),
				vec![],
			)?;
		}

		if kerns_sum != out_adjust_sum {
			return Err(Error::CoinbaseSumMismatch);
		}
		Ok(())
	}

	/// NOTE: this happens during apply_block (not the earlier validate_block)
	///
	/// Calculate lock_height as block_height + 1,000
	/// Confirm height <= lock_height
	pub fn verify_coinbase_maturity(
		&self,
		input: &Input,
		height: u64,
	) -> Result<(), Error> {
		let output = OutputIdentifier::from_input(&input);

		// We should only be calling verify_coinbase_maturity
		// if the sender claims we are spending a coinbase output
		// _and_ that we trust this claim.
		// We should have already confirmed the entry from the MMR exists
		// and has the expected hash.
		assert!(output.features.contains(COINBASE_OUTPUT));

		if let Some(_) = self.outputs
			.iter()
			.find(|x| OutputIdentifier::from_output(&x) == output)
		{
			let lock_height = self.header.height + global::coinbase_maturity();
			if lock_height > height {
				Err(Error::ImmatureCoinbase{
					height: height,
					lock_height: lock_height,
				})
			} else {
				Ok(())
			}
		} else {
			Err(Error::Other(format!("output not found in block")))
		}
	}

	/// Builds the blinded output and related signature proof for the block reward.
	pub fn reward_output(
		keychain: &keychain::Keychain,
		key_id: &keychain::Identifier,
		fees: u64,
		height: u64,
	) -> Result<(Output, TxKernel), keychain::Error> {
		let commit = keychain.commit(reward(fees), key_id)?;
		let switch_commit = keychain.switch_commit(key_id)?;
		let switch_commit_hash = SwitchCommitHash::from_switch_commit(
			switch_commit,
			keychain,
			key_id,
		);

		trace!(
			LOGGER,
			"Block reward - Pedersen Commit is: {:?}, Switch Commit is: {:?}",
			commit,
			switch_commit
		);
		trace!(
			LOGGER,
			"Block reward - Switch Commit Hash is: {:?}",
			switch_commit_hash
		);
		let msg = util::secp::pedersen::ProofMessage::empty();
		let rproof = keychain.range_proof(reward(fees), key_id, commit, msg)?;

		let output = Output {
			features: COINBASE_OUTPUT,
			commit: commit,
			switch_commit_hash: switch_commit_hash,
			proof: rproof,
		};

		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		let over_commit = secp.commit_value(reward(fees))?;
		let out_commit = output.commitment();
		let excess = secp.commit_sum(vec![out_commit], vec![over_commit])?;

		// NOTE: Remember we sign the fee *and* the lock_height.
		// For a coinbase output the fee is 0 and the lock_height is
		// the lock_height of the coinbase output itself,
		// not the lock_height of the tx (there is no tx for a coinbase output).
		// This output will not be spendable earlier than lock_height (and we sign this here).
		let msg = secp::Message::from_slice(&kernel_sig_msg(0, height))?;
		let sig = keychain.aggsig_sign_from_key_id(&msg, &key_id)?;

		let proof = TxKernel {
			features: COINBASE_KERNEL,
			excess: excess,
			excess_sig: sig,
			fee: 0,
			// lock_height here is the height of the block (tx should be valid immediately)
			// *not* the lock_height of the coinbase output (only spendable 1,000 blocks later)
			lock_height: height,
		};
		Ok((output, proof))
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use core::hash::ZERO_HASH;
	use core::Transaction;
	use core::build::{self, input, output, with_fee};
	use core::test::tx2i1o;
	use keychain::{Identifier, Keychain};
	use consensus::{MAX_BLOCK_WEIGHT, BLOCK_OUTPUT_WEIGHT};
	use std::time::Instant;

	use util::secp;

	// utility to create a block without worrying about the key or previous
	// header
	fn new_block(txs: Vec<&Transaction>, keychain: &Keychain) -> Block {
		let key_id = keychain.derive_key_id(1).unwrap();
		Block::new(
			&BlockHeader::default(),
			txs,
			keychain,
			&key_id,
			Difficulty::one()
		).unwrap()
	}

	// utility producing a transaction that spends an output with the provided
	// value and blinding key
	fn txspend1i1o(
		v: u64,
		keychain: &Keychain,
		key_id1: Identifier,
		key_id2: Identifier,
	) -> Transaction {
		build::transaction(
			vec![input(v, ZERO_HASH, key_id1), output(3, key_id2), with_fee(2)],
			&keychain,
		).map(|(tx, _)| tx)
			.unwrap()
	}

	// Too slow for now #[test]
	// TODO: make this fast enough or add similar but faster test?
	#[allow(dead_code)]
	fn too_large_block() {
		let keychain = Keychain::from_random_seed().unwrap();
		let max_out = MAX_BLOCK_WEIGHT / BLOCK_OUTPUT_WEIGHT;

		let mut pks = vec![];
		for n in 0..(max_out + 1) {
			pks.push(keychain.derive_key_id(n as u32).unwrap());
		}

		let mut parts = vec![];
		for _ in 0..max_out {
			parts.push(output(5, pks.pop().unwrap()));
		}

		let now = Instant::now();
		parts.append(&mut vec![input(500000, ZERO_HASH, pks.pop().unwrap()), with_fee(2)]);
		let mut tx = build::transaction(parts, &keychain)
			.map(|(tx, _)| tx)
			.unwrap();
		println!("Build tx: {}", now.elapsed().as_secs());

		let b = new_block(vec![&mut tx], &keychain);
		assert!(b.validate().is_err());
	}

	#[test]
	// block with no inputs/outputs/kernels
	// no fees, no reward, no coinbase
	fn very_empty_block() {
		let b = Block {
			header: BlockHeader::default(),
			inputs: vec![],
			outputs: vec![],
			kernels: vec![],
		};

		assert_eq!(
			b.verify_coinbase(),
			Err(Error::Secp(secp::Error::IncorrectCommitSum))
		);

	}

	#[test]
	// builds a block with a tx spending another and check that cut_through occurred
	fn block_with_cut_through() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let key_id3 = keychain.derive_key_id(3).unwrap();

		let mut btx1 = tx2i1o();
		let (mut btx2, _) = build::transaction(
			vec![input(7, ZERO_HASH, key_id1), output(5, key_id2.clone()), with_fee(2)],
			&keychain,
		).unwrap();

		// spending tx2 - reuse key_id2

		let mut btx3 = txspend1i1o(5, &keychain, key_id2.clone(), key_id3);
		let b = new_block(vec![&mut btx1, &mut btx2, &mut btx3], &keychain);

		// block should have been automatically compacted (including reward
		// output) and should still be valid
		b.validate().unwrap();
		assert_eq!(b.inputs.len(), 3);
		assert_eq!(b.outputs.len(), 3);
	}

	#[test]
	// builds 2 different blocks with a tx spending another and check if merging
	// occurs
	fn mergeable_blocks() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let key_id3 = keychain.derive_key_id(3).unwrap();

		let mut btx1 = tx2i1o();

		let (mut btx2, _) = build::transaction(
			vec![input(7, ZERO_HASH, key_id1), output(5, key_id2.clone()), with_fee(2)],
			&keychain,
		).unwrap();

		// spending tx2 - reuse key_id2
		let mut btx3 = txspend1i1o(5, &keychain, key_id2.clone(), key_id3);

		let b1 = new_block(vec![&mut btx1, &mut btx2], &keychain);
		b1.validate().unwrap();

		let b2 = new_block(vec![&mut btx3], &keychain);
		b2.validate().unwrap();

		// block should have been automatically compacted and should still be valid
		let b3 = b1.merge(b2);
		assert_eq!(b3.inputs.len(), 3);
		assert_eq!(b3.outputs.len(), 4);
		assert_eq!(b3.kernels.len(), 5);

		// The merged block will actually fail validation (>1 coinbase kernels)
		// Technically we support blocks with multiple coinbase kernels but we are
		// artificially limiting it to 1 for now
		assert!(b3.validate().is_err());
		assert_eq!(
			b3.kernels
				.iter()
				.filter(|x| x.features.contains(COINBASE_KERNEL))
				.count(),
			2);
	}

	#[test]
	fn empty_block_with_coinbase_is_valid() {
		let keychain = Keychain::from_random_seed().unwrap();
		let b = new_block(vec![], &keychain);

		assert_eq!(b.inputs.len(), 0);
		assert_eq!(b.outputs.len(), 1);
		assert_eq!(b.kernels.len(), 1);

		let coinbase_outputs = b.outputs
			.iter()
			.filter(|out| out.features.contains(COINBASE_OUTPUT))
			.map(|o| o.clone())
			.collect::<Vec<_>>();
		assert_eq!(coinbase_outputs.len(), 1);

		let coinbase_kernels = b.kernels
			.iter()
			.filter(|out| out.features.contains(COINBASE_KERNEL))
			.map(|o| o.clone())
			.collect::<Vec<_>>();
		assert_eq!(coinbase_kernels.len(), 1);

		// the block should be valid here (single coinbase output with corresponding
		// txn kernel)
		assert_eq!(b.validate(), Ok(()));
	}

	#[test]
	// test that flipping the COINBASE_OUTPUT flag on the output features
	// invalidates the block and specifically it causes verify_coinbase to fail
	// additionally verifying the merkle_inputs_outputs also fails
	fn remove_coinbase_output_flag() {
		let keychain = Keychain::from_random_seed().unwrap();
		let mut b = new_block(vec![], &keychain);

		assert!(b.outputs[0].features.contains(COINBASE_OUTPUT));
		b.outputs[0].features.remove(COINBASE_OUTPUT);

		assert_eq!(
			b.verify_coinbase(),
			Err(Error::CoinbaseSumMismatch)
		);
		assert_eq!(b.verify_kernels(), Ok(()));

		assert_eq!(
			b.validate(),
			Err(Error::CoinbaseSumMismatch)
		);
	}

	#[test]
	// test that flipping the COINBASE_KERNEL flag on the kernel features
	// invalidates the block and specifically it causes verify_coinbase to fail
	fn remove_coinbase_kernel_flag() {
		let keychain = Keychain::from_random_seed().unwrap();
		let mut b = new_block(vec![], &keychain);

		assert!(b.kernels[0].features.contains(COINBASE_KERNEL));
		b.kernels[0].features.remove(COINBASE_KERNEL);

		assert_eq!(
			b.verify_coinbase(),
			Err(Error::Secp(secp::Error::IncorrectCommitSum))
		);

		assert_eq!(
			b.validate(),
			Err(Error::Secp(secp::Error::IncorrectCommitSum))
		);
	}

	#[test]
	fn serialize_deserialize_block() {
		let keychain = Keychain::from_random_seed().unwrap();
		let b = new_block(vec![], &keychain);

		let mut vec = Vec::new();
		ser::serialize(&mut vec, &b).expect("serialization failed");
		let b2: Block = ser::deserialize(&mut &vec[..]).unwrap();

		assert_eq!(b.header, b2.header);
		assert_eq!(b.inputs, b2.inputs);
		assert_eq!(b.outputs, b2.outputs);
		assert_eq!(b.kernels, b2.kernels);
	}

	#[test]
	fn convert_block_to_compact_block() {
		let keychain = Keychain::from_random_seed().unwrap();
		let tx1 = tx2i1o();
		let b = new_block(vec![&tx1], &keychain);

		let cb = b.as_compact_block();

		assert_eq!(cb.kern_full.len(), 1);
		assert_eq!(cb.kern_ids.len(), 1);
		assert_eq!(cb.out_full.len(), 1);
		assert_eq!(cb.out_ids.len(), 1);
		assert_eq!(cb.in_ids.len(), 2);
		assert_eq!(
			cb.out_ids[0],
			b.outputs
				.iter()
				.find(|x| !x.features.contains(COINBASE_OUTPUT))
				.unwrap()
				.short_id(&b.hash())
		);
		assert_eq!(
			cb.kern_ids[0],
			b.kernels
				.iter()
				.find(|x| !x.features.contains(COINBASE_KERNEL))
				.unwrap()
				.short_id(&b.hash())
		);
	}

	#[test]
	fn serialize_deserialize_compact_block() {
		let b = CompactBlock {
			header: BlockHeader::default(),
			out_full: vec![],
			kern_full: vec![],
			in_ids: vec![ShortId::zero(), ShortId::zero()],
			out_ids: vec![ShortId::zero(), ShortId::zero(), ShortId::zero()],
			kern_ids: vec![ShortId::zero()],
		};

		let mut vec = Vec::new();
		ser::serialize(&mut vec, &b).expect("serialization failed");
		let b2: CompactBlock = ser::deserialize(&mut &vec[..]).unwrap();

		assert_eq!(b.header, b2.header);
		assert_eq!(b.in_ids, b2.in_ids);
		assert_eq!(b.out_ids, b2.out_ids);
		assert_eq!(b.kern_ids, b2.kern_ids);
	}
}
