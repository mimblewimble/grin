// Copyright 2016 The Developers
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
use secp;
use secp::{Secp256k1, Signature, Message};
use secp::key::SecretKey;
use std::collections::HashSet;

use core::Committed;
use core::{Input, Output, Proof, TxProof, Transaction};
use core::transaction::merkle_inputs_outputs;
use core::{PROOFSIZE, REWARD};
use core::hash::{Hash, Hashed, ZERO_HASH};
use core::transaction::MAX_IN_OUT_LEN;
use ser::{self, Readable, Reader, Writeable, Writer};

/// Block header, fairly standard compared to other blockchains.
pub struct BlockHeader {
	pub height: u64,
	pub previous: Hash,
	pub timestamp: time::Tm,
	pub td: u64, // total difficulty up to this block
	pub utxo_merkle: Hash,
	pub tx_merkle: Hash,
	pub total_fees: u64,
	pub nonce: u64,
	pub pow: Proof,
}

impl Default for BlockHeader {
	fn default() -> BlockHeader {
		BlockHeader {
			height: 0,
			previous: ZERO_HASH,
			timestamp: time::at_utc(time::Timespec { sec: 0, nsec: 0 }),
			td: 0,
			utxo_merkle: ZERO_HASH,
			tx_merkle: ZERO_HASH,
			total_fees: 0,
			nonce: 0,
			pow: Proof::zero(),
		}
	}
}

// Only Writeable implementation is required for hashing, which is part of
// core. Readable is in the ser package.
impl Writeable for BlockHeader {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		ser_multiwrite!(writer,
		                [write_u64, self.height],
		                [write_fixed_bytes, &self.previous],
		                [write_i64, self.timestamp.to_timespec().sec],
		                [write_fixed_bytes, &self.utxo_merkle],
		                [write_fixed_bytes, &self.tx_merkle],
		                [write_u64, self.total_fees]);
		// make sure to not introduce any variable length data before the nonce to
		// avoid complicating PoW
		try!(writer.write_u64(self.nonce));
		// cuckoo cycle of 42 nodes
		for n in 0..42 {
			try!(writer.write_u32(self.pow.0[n]));
		}
		writer.write_u64(self.td)
	}
}

/// A block as expressed in the MimbleWimble protocol. The reward is
/// non-explicit, assumed to be deductible from block height (similar to
/// bitcoin's schedule) and expressed as a global transaction fee (added v.H),
/// additive to the total of fees ever collected.
pub struct Block {
	// hash_mem: Hash,
	pub header: BlockHeader,
	pub inputs: Vec<Input>,
	pub outputs: Vec<Output>,
	pub proofs: Vec<TxProof>,
}

/// Implementation of Writeable for a block, defines how to write the full
/// block as binary.
impl Writeable for Block {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		try!(self.header.write(writer));

		ser_multiwrite!(writer,
		                [write_u64, self.inputs.len() as u64],
		                [write_u64, self.outputs.len() as u64],
		                [write_u64, self.proofs.len() as u64]);
		for inp in &self.inputs {
			try!(inp.write(writer));
		}
		for out in &self.outputs {
			try!(out.write(writer));
		}
		for proof in &self.proofs {
			try!(proof.write(writer));
		}
		Ok(())
	}
}

/// Implementation of Readable for a block, defines how to read a full block
/// from a binary stream.
impl Readable<Block> for Block {
	fn read(reader: &mut Reader) -> Result<Block, ser::Error> {
		let (height, previous, timestamp, utxo_merkle, tx_merkle, total_fees, nonce) =
			ser_multiread!(reader,
			               read_u64,
			               read_32_bytes,
			               read_i64,
			               read_32_bytes,
			               read_32_bytes,
			               read_u64,
			               read_u64);

		// cuckoo cycle of 42 nodes
		let mut pow = [0; PROOFSIZE];
		for n in 0..PROOFSIZE {
			pow[n] = try!(reader.read_u32());
		}

		let (td, input_len, output_len, proof_len) =
			ser_multiread!(reader, read_u64, read_u64, read_u64, read_u64);

		if input_len > MAX_IN_OUT_LEN || output_len > MAX_IN_OUT_LEN || proof_len > MAX_IN_OUT_LEN {
			return Err(ser::Error::TooLargeReadErr("Too many inputs, outputs or proofs."
				.to_string()));
		}

		let inputs = try!((0..input_len).map(|_| Input::read(reader)).collect());
		let outputs = try!((0..output_len).map(|_| Output::read(reader)).collect());
		let proofs = try!((0..proof_len).map(|_| TxProof::read(reader)).collect());

		Ok(Block {
			header: BlockHeader {
				height: height,
				previous: Hash::from_vec(previous),
				timestamp: time::at_utc(time::Timespec {
					sec: timestamp,
					nsec: 0,
				}),
				td: td,
				utxo_merkle: Hash::from_vec(utxo_merkle),
				tx_merkle: Hash::from_vec(tx_merkle),
				total_fees: total_fees,
				pow: Proof(pow),
				nonce: nonce,
			},
			inputs: inputs,
			outputs: outputs,
			proofs: proofs,
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
		(REWARD as i64) - (self.header.total_fees as i64)
	}
}

/// Default properties for a block, everything zeroed out and empty vectors.
impl Default for Block {
	fn default() -> Block {
		Block {
			header: Default::default(),
			inputs: vec![],
			outputs: vec![],
			proofs: vec![],
		}
	}
}

impl Block {
	/// Builds a new block from the header of the previous block, a vector of
	/// transactions and the private key that will receive the reward. Checks
	/// that all transactions are valid and calculates the Merkle tree.
	pub fn new(prev: BlockHeader,
	           txs: Vec<&mut Transaction>,
	           reward_key: SecretKey)
	           -> Result<Block, secp::Error> {

		let secp = Secp256k1::with_caps(secp::ContextFlag::Commit);
		let (reward_out, reward_proof) = try!(Block::reward_output(reward_key, &secp));

		// note: the following reads easily but may not be the most efficient due to
		// repeated iterations, revisit if a problem

		// validate each transaction and gather their proofs
		let mut proofs = try_map_vec!(txs, |tx| tx.verify_sig(&secp));
		proofs.push(reward_proof);

		// build vectors with all inputs and all outputs, ordering them by hash
		// needs to be a fold so we don't end up with a vector of vectors and we
		// want to fullt own the refs (not just a pointer like flat_map).
		let mut inputs = txs.iter()
			.fold(vec![], |mut acc, ref tx| {
				let mut inputs = tx.inputs.clone();
				acc.append(&mut inputs);
				acc
			});
		let mut outputs = txs.iter()
			.fold(vec![], |mut acc, ref tx| {
				let mut outputs = tx.outputs.clone();
				acc.append(&mut outputs);
				acc
			});
		outputs.push(reward_out);

		inputs.sort_by_key(|inp| inp.hash());
		outputs.sort_by_key(|out| out.hash());

		// calculate the overall Merkle tree and fees
		let fees = txs.iter().map(|tx| tx.fee).sum();

		Ok(Block {
				header: BlockHeader {
					height: prev.height + 1,
					total_fees: fees,
					timestamp: time::now(),
					..Default::default()
				},
				inputs: inputs,
				outputs: outputs,
				proofs: proofs,
			}
			.compact())
	}

	pub fn hash(&self) -> Hash {
		self.header.hash()
	}

	/// Matches any output with a potential spending input, eliminating them
	/// from the block. Provides a simple way to compact the block. The
	/// elimination is stable with respect to inputs and outputs order.
	pub fn compact(&self) -> Block {
		// the chosen ones
		let mut new_inputs = vec![];

		// build a set of all output hashes
		let mut out_set = HashSet::new();
		for out in &self.outputs {
			out_set.insert(out.hash());
		}
		// removes from the set any hash referenced by an input, keeps the inputs that
		// don't have a match
		for inp in &self.inputs {
			if !out_set.remove(&inp.output_hash()) {
				new_inputs.push(*inp);
			}
		}
		// we got ourselves a keep list in that set
		let new_outputs = self.outputs
			.iter()
			.filter(|out| out_set.contains(&(out.hash())))
			.map(|&out| out)
			.collect::<Vec<Output>>();

		let tx_merkle = merkle_inputs_outputs(&new_inputs, &new_outputs);

		Block {
			header: BlockHeader {
				tx_merkle: tx_merkle,
				pow: self.header.pow.clone(),
				..self.header
			},
			inputs: new_inputs,
			outputs: new_outputs,
			proofs: self.proofs.clone(),
		}
	}

	// Merges the 2 blocks, essentially appending the inputs, outputs and proofs.
	// Also performs a compaction on the result.
	pub fn merge(&self, other: Block) -> Block {
		let mut all_inputs = self.inputs.clone();
		all_inputs.append(&mut other.inputs.clone());

		let mut all_outputs = self.outputs.clone();
		all_outputs.append(&mut other.outputs.clone());

		let mut all_proofs = self.proofs.clone();
		all_proofs.append(&mut other.proofs.clone());

		all_inputs.sort_by_key(|inp| inp.hash());
		all_outputs.sort_by_key(|out| out.hash());

		Block {
				// compact will fix the merkle tree
				header: BlockHeader {
					total_fees: self.header.total_fees + other.header.total_fees,
					pow: self.header.pow.clone(),
					..self.header
				},
				inputs: all_inputs,
				outputs: all_outputs,
				proofs: all_proofs,
			}
			.compact()
	}

	/// Checks the block is valid by verifying the overall commitments sums and
	/// proofs.
	pub fn verify(&self, secp: &Secp256k1) -> Result<(), secp::Error> {
		// sum all inputs and outs commitments
		let io_sum = try!(self.sum_commitments(secp));
		// sum all proofs commitments
		let proof_commits = map_vec!(self.proofs, |proof| proof.remainder);
		let proof_sum = try!(secp.commit_sum(proof_commits, vec![]));

		// both should be the same
		if proof_sum != io_sum {
			// TODO more specific error
			return Err(secp::Error::IncorrectCommitSum);
		}

		// verify all signatures with the commitment as pk
		for proof in &self.proofs {
			try!(proof.verify(secp));
		}
		Ok(())
	}

	// Builds the blinded output and related signature proof for the block reward.
	fn reward_output(skey: secp::key::SecretKey,
	                 secp: &Secp256k1)
	                 -> Result<(Output, TxProof), secp::Error> {
		let msg = try!(secp::Message::from_slice(&[0; 32]));
		let sig = try!(secp.sign(&msg, &skey));
		let output = Output::OvertOutput {
				value: REWARD,
				blindkey: skey,
			}
			.blind(&secp);

		let over_commit = try!(secp.commit_value(REWARD as u64));
		let out_commit = output.commitment().unwrap();
		let remainder = try!(secp.commit_sum(vec![over_commit], vec![out_commit]));

		let proof = TxProof {
			remainder: remainder,
			sig: sig.serialize_der(&secp),
      fee: 0,
		};
		Ok((output, proof))
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use core::{Input, Output, Transaction};
	use core::hash::{Hash, Hashed};
	use core::test::{tx1i1o, tx2i1o};

	use secp::{self, Secp256k1};
	use secp::key::SecretKey;
	use rand::Rng;
	use rand::os::OsRng;

	fn new_secp() -> Secp256k1 {
		secp::Secp256k1::with_caps(secp::ContextFlag::Commit)
	}

	// utility to create a block without worrying about the key or previous header
	fn new_block(txs: Vec<&mut Transaction>, secp: &Secp256k1) -> Block {
		let mut rng = OsRng::new().unwrap();
		let skey = SecretKey::new(secp, &mut rng);
		Block::new(BlockHeader::default(), txs, skey).unwrap()
	}

	// utility producing a transaction that spends the above
	fn txspend1i1o<R: Rng>(secp: &Secp256k1, rng: &mut R, oout: Output, outh: Hash) -> Transaction {
		if let Output::OvertOutput { blindkey, value } = oout {
			Transaction::new(vec![Input::OvertInput {
				                      output: outh,
				                      value: value,
				                      blindkey: blindkey,
			                      }],
			                 vec![Output::OvertOutput {
				                      value: 3,
				                      blindkey: SecretKey::new(secp, rng),
			                      }],
			                 1)
		} else {
			panic!();
		}
	}

	#[test]
	// builds a block with a tx spending another and check if merging occurred
	fn compactable_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();

		let tx1 = tx2i1o(secp, &mut rng);
		let mut btx1 = tx1.blind(&secp).unwrap();

		let tx2 = tx1i1o(secp, &mut rng);
		let mut btx2 = tx2.blind(&secp).unwrap();

		// spending tx2
		let spending = txspend1i1o(secp, &mut rng, tx2.outputs[0], btx2.outputs[0].hash());
		let mut btx3 = spending.blind(&secp).unwrap();
		let b = new_block(vec![&mut btx1, &mut btx2, &mut btx3], secp);

		// block should have been automatically compacted (including reward output) and
		// should still be valid
		b.verify(&secp).unwrap();
		assert_eq!(b.inputs.len(), 3);
		assert_eq!(b.outputs.len(), 3);
	}

	#[test]
	// builds 2 different blocks with a tx spending another and check if merging
	// occurs
	fn mergeable_blocks() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();

		let tx1 = tx2i1o(secp, &mut rng);
		let mut btx1 = tx1.blind(&secp).unwrap();

		let tx2 = tx1i1o(secp, &mut rng);
		let mut btx2 = tx2.blind(&secp).unwrap();

		// spending tx2
		let spending = txspend1i1o(secp, &mut rng, tx2.outputs[0], btx2.outputs[0].hash());
		let mut btx3 = spending.blind(&secp).unwrap();

		let b1 = new_block(vec![&mut btx1, &mut btx2], secp);
		b1.verify(&secp).unwrap();
		let b2 = new_block(vec![&mut btx3], secp);
		b2.verify(&secp).unwrap();

		// block should have been automatically compacted and should still be valid
		let b3 = b1.merge(b2);
		assert_eq!(b3.inputs.len(), 3);
		assert_eq!(b3.outputs.len(), 4);
	}
}
