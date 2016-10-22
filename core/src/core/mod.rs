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

//! Core types

#[allow(dead_code)]
#[macro_use]
mod ser;
use ser::{Writeable, Writer, Error, ser_vec};

use time;

use std::fmt;
use std::cmp::Ordering;
use std::collections::HashSet;

use secp;
use secp::{Secp256k1, Signature, Message};
use secp::key::SecretKey;
use secp::pedersen::*;

use tiny_keccak::Keccak;

/// The block subsidy amount
pub const REWARD: u64 = 1_000_000_000;

/// Block interval, in seconds
pub const BLOCK_TIME_SEC: u8 = 15;

/// Cuckoo-cycle proof size (cycle length)
pub const PROOFSIZE: usize = 42;

/// A hash to uniquely (or close enough) identify one of the main blockchain
/// constructs. Used pervasively for blocks, transactions and ouputs.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct Hash(pub [u8; 32]);

impl fmt::Display for Hash {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		for i in self.0[..].iter().cloned() {
			try!(write!(f, "{:02x}", i));
		}
		Ok(())
	}
}

impl Hash {
	/// Creates a new hash from a vector
	pub fn from_vec(v: Vec<u8>) -> Hash {
		let mut a = [0; 32];
		for i in 0..a.len() {
			a[i] = v[i];
		}
		Hash(a)
	}
	/// Converts the hash to a byte vector
	pub fn to_vec(&self) -> Vec<u8> {
		self.0.to_vec()
	}
	/// Converts the hash to a byte slice
	pub fn to_slice(&self) -> &[u8] {
		&self.0
	}
}

pub const ZERO_HASH: Hash = Hash([0; 32]);

/// A trait for types that get their hash (double SHA256) from their byte
/// serialzation.
pub trait Hashed {
	fn hash(&self) -> Hash {
		let data = self.bytes();
		Hash(sha3(data))
	}

	fn bytes(&self) -> Vec<u8>;
}

fn sha3(data: Vec<u8>) -> [u8; 32] {
	let mut sha3 = Keccak::new_sha3_256();
	let mut buf = [0; 32];
	sha3.update(&data);
	sha3.finalize(&mut buf);
	buf
}

/// Implemented by types that hold inputs and outputs including Pedersen
/// commitments. Handles the collection of the commitments as well as their
/// summing, taking potential explicit overages of fees into account.
pub trait Committed {
	/// Gathers commitments and sum them.
	fn sum_commitments(&self, secp: &Secp256k1) -> Result<Commitment, secp::Error> {
		// first, verify each range proof
		let ref outputs = self.outputs_committed();
		for output in *outputs {
			try!(output.verify_proof(secp))
		}

		// then gather the commitments
		let mut input_commits = filter_map_vec!(self.inputs_committed(), |inp| inp.commitment());
		let mut output_commits = filter_map_vec!(self.outputs_committed(), |out| out.commitment());

		// add the overage as input commitment if positive, as an output commitment if
		// negative
		let overage = self.overage();
		if overage != 0 {
			let over_commit = secp.commit_value(overage.abs() as u64).unwrap();
			if overage > 0 {
				input_commits.push(over_commit);
			} else {
				output_commits.push(over_commit);
			}
		}

		// sum all that stuff
		secp.commit_sum(input_commits, output_commits)
	}

	/// Vector of committed inputs to verify
	fn inputs_committed(&self) -> &Vec<Input>;

	/// Vector of committed inputs to verify
	fn outputs_committed(&self) -> &Vec<Output>;

	/// The overage amount expected over the commitments. Can be negative (a
	/// fee) or positive (a reward).
	fn overage(&self) -> i64;
}

/// A proof that a transaction did not create (or remove) funds. Includes both
/// the transaction's Pedersen commitment and the signature that guarantees
/// that the commitment amounts to zero.
#[derive(Debug, Clone)]
pub struct TxProof {
	remainder: Commitment,
	sig: Vec<u8>,
}

/// Proof of work
#[derive(Copy)]
pub struct Proof(pub [u32; PROOFSIZE]);

impl fmt::Debug for Proof {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		try!(write!(f, "Cuckoo("));
		for (i, val) in self.0[..].iter().enumerate() {
			try!(write!(f, "{:x}", val));
			if i < PROOFSIZE - 1 {
				write!(f, " ");
			}
		}
		write!(f, ")")
	}
}
impl PartialOrd for Proof {
	fn partial_cmp(&self, other: &Proof) -> Option<Ordering> {
		self.0.partial_cmp(&other.0)
	}
}
impl PartialEq for Proof {
	fn eq(&self, other: &Proof) -> bool {
		self.0[..] == other.0[..]
	}
}
impl Eq for Proof {}
impl Clone for Proof {
	fn clone(&self) -> Proof {
		*self
	}
}

impl Proof {
	/// Builds a proof with all bytes zeroed out
	pub fn zero() -> Proof {
		Proof([0; 42])
	}
	/// Builds a proof from a vector of exactly PROOFSIZE (42) u32.
	pub fn from_vec(v: Vec<u32>) -> Proof {
		assert!(v.len() == PROOFSIZE);
		let mut p = [0; PROOFSIZE];
		for (n, elem) in v.iter().enumerate() {
			p[n] = *elem;
		}
		Proof(p)
	}
	/// Converts the proof to a vector of u64s
	pub fn to_u64s(&self) -> Vec<u64> {
		let mut nonces = Vec::with_capacity(PROOFSIZE);
		for n in self.0.iter() {
			nonces.push(*n as u64);
		}
		nonces
	}
	/// Converts the proof to a vector of u32s
	pub fn to_u32s(&self) -> Vec<u32> {
		self.0.to_vec()
	}
}

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
	fn write(&self, writer: &mut Writer) -> Option<Error> {
		try_m!(writer.write_u64(self.height));
		try_m!(writer.write_fixed_bytes(&self.previous));
		try_m!(writer.write_i64(self.timestamp.to_timespec().sec));
		try_m!(writer.write_fixed_bytes(&self.utxo_merkle));
		try_m!(writer.write_fixed_bytes(&self.tx_merkle));
		try_m!(writer.write_u64(self.total_fees));
		// make sure to not introduce any variable length data before the nonce to
		// avoid complicating PoW
		try_m!(writer.write_u64(self.nonce));
		// cuckoo cycle of 42 nodes
		for n in 0..42 {
			try_m!(writer.write_u32(self.pow.0[n]));
		}
		writer.write_u64(self.td)
	}
}

impl Hashed for BlockHeader {
	fn bytes(&self) -> Vec<u8> {
		// no serialization errors are applicable in this specific case
		ser_vec(self).unwrap()
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

		let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
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
		let msg = try!(Message::from_slice(&[0; 32]));
		for proof in &self.proofs {
			let pubk = try!(proof.remainder.to_pubkey(secp));
			let sig = try!(Signature::from_der(secp, &proof.sig));
			try!(secp.verify(&msg, &sig, &pubk));
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
		};
		Ok((output, proof))
	}
}

#[derive(Debug)]
pub struct Transaction {
	hash_mem: Option<Hash>,
	pub fee: u64,
	pub zerosig: Vec<u8>,
	pub inputs: Vec<Input>,
	pub outputs: Vec<Output>,
}

impl Committed for Transaction {
	fn inputs_committed(&self) -> &Vec<Input> {
		&self.inputs
	}
	fn outputs_committed(&self) -> &Vec<Output> {
		&self.outputs
	}
	fn overage(&self) -> i64 {
		-(self.fee as i64)
	}
}

impl Default for Transaction {
	fn default() -> Transaction {
		Transaction::empty()
	}
}

impl Transaction {
	/// Creates a new empty transaction (no inputs or outputs, zero fee).
	pub fn empty() -> Transaction {
		Transaction {
			hash_mem: None,
			fee: 0,
			zerosig: vec![],
			inputs: vec![],
			outputs: vec![],
		}
	}

	/// Creates a new transaction initialized with the provided inputs,
	/// outputs and fee.
	pub fn new(inputs: Vec<Input>, outputs: Vec<Output>, fee: u64) -> Transaction {
		Transaction {
			hash_mem: None,
			fee: fee,
			zerosig: vec![],
			inputs: inputs,
			outputs: outputs,
		}
	}

	/// The hash of a transaction is the Merkle tree of its inputs and outputs
	/// hashes. None of the rest is required.
	fn hash(&mut self) -> Hash {
		if let None = self.hash_mem {
			self.hash_mem = Some(merkle_inputs_outputs(&self.inputs, &self.outputs));
		}
		self.hash_mem.unwrap()
	}

	/// Takes a transaction and fully blinds it. Following the MW
	/// algorithm: calculates the commitments for each inputs and outputs
	/// using the values and blinding factors, takes the blinding factors
	/// remainder and uses it for an empty signature.
	pub fn blind(&self, secp: &Secp256k1) -> Result<Transaction, secp::Error> {
		// we compute the sum of blinding factors to get the k remainder
		let remainder = try!(self.blind_sum(secp));

		// next, blind the inputs and outputs if they haven't been yet
		let blind_inputs = map_vec!(self.inputs, |inp| inp.blind(secp));
		let blind_outputs = map_vec!(self.outputs, |out| out.blind(secp));

		// and sign with the remainder so the signature can be checked to match with
		// the k.G commitment leftover, that should also be the pubkey
		let msg = try!(Message::from_slice(&[0; 32]));
		let sig = try!(secp.sign(&msg, &remainder));

		Ok(Transaction {
			hash_mem: None,
			fee: self.fee,
			zerosig: sig.serialize_der(secp),
			inputs: blind_inputs,
			outputs: blind_outputs,
		})
	}

	/// Compute the sum of blinding factors on all overt inputs and outputs
	/// of the transaction to get the k remainder.
	pub fn blind_sum(&self, secp: &Secp256k1) -> Result<SecretKey, secp::Error> {
		let inputs_blinding_fact = filter_map_vec!(self.inputs, |inp| inp.blinding_factor());
		let outputs_blinding_fact = filter_map_vec!(self.outputs, |out| out.blinding_factor());

		secp.blind_sum(inputs_blinding_fact, outputs_blinding_fact)
	}

	/// The verification for a MimbleWimble transaction involves getting the
	/// remainder of summing all commitments and using it as a public key
	/// to verify the embedded signature. The rational is that if the values
	/// sum to zero as they should in r.G + v.H then only k.G the remainder
	/// of the sum of r.G should be left. And r.G is the definition of a
	/// public key generated using r as a private key.
	pub fn verify_sig(&self, secp: &Secp256k1) -> Result<TxProof, secp::Error> {
		let rsum = try!(self.sum_commitments(secp));

		// pretend the sum is a public key (which it is, being of the form r.G) and
		// verify the transaction sig with it
		let pubk = try!(rsum.to_pubkey(secp));
		let msg = try!(Message::from_slice(&[0; 32]));
		let sig = try!(Signature::from_der(secp, &self.zerosig));
		try!(secp.verify(&msg, &sig, &pubk));

		Ok(TxProof {
			remainder: rsum,
			sig: self.zerosig.clone(),
		})
	}
}

/// A transaction input, mostly a reference to an output being spent by the
/// transaction.
#[derive(Debug, Copy, Clone)]
pub enum Input {
	BareInput { output: Hash },
	BlindInput { output: Hash, commit: Commitment },
	OvertInput {
		output: Hash,
		value: u64,
		blindkey: SecretKey,
	},
}
impl Input {
	pub fn commitment(&self) -> Option<Commitment> {
		match self {
			&Input::BlindInput { commit, .. } => Some(commit),
			_ => None,
		}
	}
	pub fn blind(&self, secp: &Secp256k1) -> Input {
		match self {
			&Input::OvertInput { output, value, blindkey } => {
				let commit = secp.commit(value, blindkey).unwrap();
				Input::BlindInput {
					output: output,
					commit: commit,
				}
			}
			_ => *self,
		}
	}
	pub fn blinding_factor(&self) -> Option<SecretKey> {
		match self {
			&Input::OvertInput { blindkey, .. } => Some(blindkey),
			_ => None,
		}
	}
	pub fn output_hash(&self) -> Hash {
		match self {
			&Input::BlindInput { output, .. } => output,
			&Input::OvertInput { output, .. } => output,
			&Input::BareInput { output, .. } => output,
		}
	}
}

/// The hash of an input is the hash of the output hash it references.
impl Hashed for Input {
	fn bytes(&self) -> Vec<u8> {
		self.output_hash().to_vec()
	}
}

#[derive(Debug, Copy, Clone)]
pub enum Output {
	BlindOutput {
		commit: Commitment,
		proof: RangeProof,
	},
	OvertOutput { value: u64, blindkey: SecretKey },
}
impl Output {
	pub fn commitment(&self) -> Option<Commitment> {
		match self {
			&Output::BlindOutput { commit, .. } => Some(commit),
			_ => None,
		}
	}
	pub fn proof(&self) -> Option<RangeProof> {
		match self {
			&Output::BlindOutput { proof, .. } => Some(proof),
			_ => None,
		}
	}
	pub fn blinding_factor(&self) -> Option<SecretKey> {
		match self {
			&Output::OvertOutput { blindkey, .. } => Some(blindkey),
			_ => None,
		}
	}
	pub fn blind(&self, secp: &Secp256k1) -> Output {
		match self {
			&Output::OvertOutput { value, blindkey } => {
				let commit = secp.commit(value, blindkey).unwrap();
				let rproof = secp.range_proof(0, value, blindkey, commit);
				Output::BlindOutput {
					commit: commit,
					proof: rproof,
				}
			}
			_ => *self,
		}
	}
	/// Validates the range proof using the commitment
	pub fn verify_proof(&self, secp: &Secp256k1) -> Result<(), secp::Error> {
		match self {
			&Output::BlindOutput { commit, proof } => {
				secp.verify_range_proof(commit, proof).map(|_| ())
			}
			_ => Ok(()),
		}
	}
}

/// The hash of an output is the hash of its commitment.
impl Hashed for Output {
	fn bytes(&self) -> Vec<u8> {
		if let &Output::BlindOutput { commit, .. } = self {
			return commit.bytes().to_vec();
		} else {
			panic!("cannot hash an overt output");
		}
	}
}

/// Utility function to calculate the Merkle root of vectors of inputs and
/// outputs.
pub fn merkle_inputs_outputs(inputs: &Vec<Input>, outputs: &Vec<Output>) -> Hash {
	let mut all_hs = map_vec!(inputs, |inp| inp.hash());
	all_hs.append(&mut map_vec!(outputs, |out| out.hash()));
	MerkleRow::new(all_hs).root()
}

/// Two hashes that will get hashed together in a Merkle tree to build the next
/// level up.
struct HPair(Hash, Hash);
impl Hashed for HPair {
	fn bytes(&self) -> Vec<u8> {
		let mut data = Vec::with_capacity(64);
		data.extend_from_slice(self.0.to_slice());
		data.extend_from_slice(self.1.to_slice());
		return data;
	}
}
/// An iterator over hashes in a vector that pairs them to build a row in a
/// Merkle tree. If the vector has an odd number of hashes, duplicates the last.
struct HPairIter(Vec<Hash>);
impl Iterator for HPairIter {
	type Item = HPair;

	fn next(&mut self) -> Option<HPair> {
		self.0.pop().map(|first| HPair(first, self.0.pop().unwrap_or(first)))
	}
}
/// A row in a Merkle tree. Can be built from a vector of hashes. Calculates
/// the next level up, or can recursively go all the way up to its root.
struct MerkleRow(Vec<HPair>);
impl MerkleRow {
	fn new(hs: Vec<Hash>) -> MerkleRow {
		MerkleRow(HPairIter(hs).map(|hp| hp).collect())
	}
	fn up(&self) -> MerkleRow {
		MerkleRow::new(map_vec!(self.0, |hp| hp.hash()))
	}
	fn root(&self) -> Hash {
		if self.0.len() == 0 {
			Hash(sha3(vec![]))
		} else if self.0.len() == 1 {
			self.0[0].hash()
		} else {
			self.up().root()
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use secp;
	use secp::Secp256k1;
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

	#[test]
	fn blind_overt_output() {
		let ref secp = new_secp();
		let mut rng = OsRng::new().unwrap();

		let oo = Output::OvertOutput {
			value: 42,
			blindkey: SecretKey::new(secp, &mut rng),
		};
		if let Output::BlindOutput { commit, proof } = oo.blind(secp) {
			// checks the blind output is sane and verifies
			assert!(commit.len() > 0);
			assert!(proof.bytes().len() > 5000);
			secp.verify_range_proof(commit, proof).unwrap();

			// checks that changing the value changes the proof and commitment
			let oo2 = Output::OvertOutput {
				value: 32,
				blindkey: SecretKey::new(secp, &mut rng),
			};
			if let Output::BlindOutput { commit: c2, proof: p2 } = oo2.blind(secp) {
				assert!(c2 != commit);
				assert!(p2.bytes() != proof.bytes());
				secp.verify_range_proof(c2, p2).unwrap();

				// checks that swapping the proofs fails the validation
				if let Ok(_) = secp.verify_range_proof(commit, p2) {
					panic!("verification successful on wrong proof");
				}
			} else {
				panic!("not a blind output");
			}
		} else {
			panic!("not a blind output");
		}
	}

	#[test]
	fn hash_output() {
		let ref secp = new_secp();
		let mut rng = OsRng::new().unwrap();

		let oo = Output::OvertOutput {
				value: 42,
				blindkey: SecretKey::new(secp, &mut rng),
			}
			.blind(secp);
		let oo2 = Output::OvertOutput {
				value: 32,
				blindkey: SecretKey::new(secp, &mut rng),
			}
			.blind(secp);
		let h = oo.hash();
		assert!(h != ZERO_HASH);
		let h2 = oo2.hash();
		assert!(h != h2);
	}

	#[test]
	fn blind_tx() {
		let ref secp = new_secp();
		let mut rng = OsRng::new().unwrap();

		let tx = tx2i1o(secp, &mut rng);
		let btx = tx.blind(&secp).unwrap();
		btx.verify_sig(&secp).unwrap(); // unwrap will panic if invalid

		// checks that the range proof on our blind output is sufficiently hiding
		if let Output::BlindOutput { proof, .. } = btx.outputs[0] {
			let info = secp.range_proof_info(proof);
			assert!(info.min == 0);
			assert!(info.max == u64::max_value());
		}
	}

	#[test]
	fn tx_hash_diff() {
		let ref secp = new_secp();
		let mut rng = OsRng::new().unwrap();

		let tx1 = tx2i1o(secp, &mut rng);
		let mut btx1 = tx1.blind(&secp).unwrap();

		let tx2 = tx1i1o(secp, &mut rng);
		let mut btx2 = tx2.blind(&secp).unwrap();

		if btx1.hash() == btx2.hash() {
			panic!("diff txs have same hash")
		}
	}

	#[test]
	#[should_panic(expected = "InvalidSecretKey")]
	fn zero_commit() {
		// a transaction whose commitment sums to zero shouldn't validate
		let ref secp = new_secp();
		let mut rng = OsRng::new().unwrap();

		let skey = SecretKey::new(secp, &mut rng);
		let outh = ZERO_HASH;
		let tx = Transaction::new(vec![Input::OvertInput {
			                               output: outh,
			                               value: 10,
			                               blindkey: skey,
		                               }],
		                          vec![Output::OvertOutput {
			                               value: 1,
			                               blindkey: skey,
		                               }],
		                          9);
		// blinding should fail as signing with a zero r*G shouldn't work
		tx.blind(&secp).unwrap();
	}

	#[test]
	fn reward_empty_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();
		let skey = SecretKey::new(secp, &mut rng);

		let b = Block::new(BlockHeader::default(), vec![], skey).unwrap();
		b.compact().verify(&secp).unwrap();
	}

	#[test]
	fn reward_with_tx_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();
		let skey = SecretKey::new(secp, &mut rng);

		let tx1 = tx2i1o(secp, &mut rng);
		let mut btx1 = tx1.blind(&secp).unwrap();
		btx1.verify_sig(&secp).unwrap();

		let b = Block::new(BlockHeader::default(), vec![&mut btx1], skey).unwrap();
		b.compact().verify(&secp).unwrap();
	}

	#[test]
	fn simple_block() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();
		let skey = SecretKey::new(secp, &mut rng);

		let tx1 = tx2i1o(secp, &mut rng);
		let mut btx1 = tx1.blind(&secp).unwrap();
		btx1.verify_sig(&secp).unwrap();

		let tx2 = tx1i1o(secp, &mut rng);
		let mut btx2 = tx2.blind(&secp).unwrap();
		btx2.verify_sig(&secp).unwrap();

		let b = Block::new(BlockHeader::default(), vec![&mut btx1, &mut btx2], skey).unwrap();
		b.verify(&secp).unwrap();
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

	// utility producing a transaction with 2 inputs and a single outputs
	fn tx2i1o<R: Rng>(secp: &Secp256k1, rng: &mut R) -> Transaction {
		let outh = ZERO_HASH;
		Transaction::new(vec![Input::OvertInput {
			                      output: outh,
			                      value: 10,
			                      blindkey: SecretKey::new(secp, rng),
		                      },
		                      Input::OvertInput {
			                      output: outh,
			                      value: 11,
			                      blindkey: SecretKey::new(secp, rng),
		                      }],
		                 vec![Output::OvertOutput {
			                      value: 20,
			                      blindkey: SecretKey::new(secp, rng),
		                      }],
		                 1)
	}

	// utility producing a transaction with a single input and output
	fn tx1i1o<R: Rng>(secp: &Secp256k1, rng: &mut R) -> Transaction {
		let outh = ZERO_HASH;
		Transaction::new(vec![Input::OvertInput {
			                      output: outh,
			                      value: 5,
			                      blindkey: SecretKey::new(secp, rng),
		                      }],
		                 vec![Output::OvertOutput {
			                      value: 4,
			                      blindkey: SecretKey::new(secp, rng),
		                      }],
		                 1)
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
}
