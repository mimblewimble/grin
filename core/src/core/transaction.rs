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

//! Transactions

use byteorder::{BigEndian, ByteOrder};
use blake2::blake2b::blake2b;
use util::secp::{self, Message, Signature};
use util::static_secp_instance;
use util::secp::pedersen::{Commitment, RangeProof};
use std::cmp::Ordering;
use std::ops;

use consensus;
use core::Committed;
use core::hash::Hashed;
use core::pmmr::Summable;
use keychain::{Identifier, Keychain};
use ser::{self, read_and_verify_sorted, Readable, Reader, Writeable, WriteableSorted, Writer};

/// The size to use for the stored blake2 hash of a switch_commitment
pub const SWITCH_COMMIT_HASH_SIZE: usize = 20;

bitflags! {
	/// Options for a kernel's structure or use
	pub flags KernelFeatures: u8 {
		/// No flags
		const DEFAULT_KERNEL = 0b00000000,
		/// Kernel matching a coinbase output
		const COINBASE_KERNEL = 0b00000001,
	}
}

// don't seem to be able to define an Ord implementation for Hash due to
// Ord being defined on all pointers, resorting to a macro instead
macro_rules! hashable_ord {
  ($hashable: ident) => {
    impl Ord for $hashable {
      fn cmp(&self, other: &$hashable) -> Ordering {
        self.hash().cmp(&other.hash())
      }
    }
    impl PartialOrd for $hashable {
      fn partial_cmp(&self, other: &$hashable) -> Option<Ordering> {
        Some(self.hash().cmp(&other.hash()))
      }
    }
    impl PartialEq for $hashable {
      fn eq(&self, other: &$hashable) -> bool {
        self.hash() == other.hash()
      }
    }
    impl Eq for $hashable {}
  }
}

/// Errors thrown by Block validation
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
	/// Transaction fee can't be odd, due to half fee burning
	OddFee,
	/// Underlying Secp256k1 error (signature validation or invalid public
	/// key typically)
	Secp(secp::Error),
	/// Restrict number of incoming inputs
	TooManyInputs,
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

/// Construct msg bytes from tx fee and lock_height
pub fn kernel_sig_msg(fee: u64, lock_height: u64) -> [u8; 32] {
	let mut bytes = [0; 32];
	BigEndian::write_u64(&mut bytes[16..24], fee);
	BigEndian::write_u64(&mut bytes[24..], lock_height);
	bytes
}

/// A proof that a transaction sums to zero. Includes both the transaction's
/// Pedersen commitment and the signature, that guarantees that the commitments
/// amount to zero.
/// The signature signs the fee and the lock_height, which are retained for
/// signature validation.
#[derive(Debug, Clone)]
pub struct TxKernel {
	/// Options for a kernel's structure or use
	pub features: KernelFeatures,
	/// Fee originally included in the transaction this proof is for.
	pub fee: u64,
	/// This kernel is not valid earlier than lock_height blocks
	/// The max lock_height of all *inputs* to this transaction
	pub lock_height: u64,
	/// Remainder of the sum of all transaction commitments. If the transaction
	/// is well formed, amounts components should sum to zero and the excess
	/// is hence a valid public key.
	pub excess: Commitment,
	/// The signature proving the excess is a valid public key, which signs
	/// the transaction fee.
	pub excess_sig: Vec<u8>,
}

hashable_ord!(TxKernel);

impl Writeable for TxKernel {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u8, self.features.bits()],
			[write_u64, self.fee],
			[write_u64, self.lock_height],
			[write_fixed_bytes, &self.excess],
			[write_bytes, &self.excess_sig]
		);
		Ok(())
	}
}

impl Readable for TxKernel {
	fn read(reader: &mut Reader) -> Result<TxKernel, ser::Error> {
		let features = KernelFeatures::from_bits(reader.read_u8()?).ok_or(
			ser::Error::CorruptedData,
		)?;

		Ok(TxKernel {
			features: features,
			fee: reader.read_u64()?,
			lock_height: reader.read_u64()?,
			excess: Commitment::read(reader)?,
			excess_sig: reader.read_vec()?,
		})
	}
}

impl TxKernel {
	/// Verify the transaction proof validity. Entails handling the commitment
	/// as a public key and checking the signature verifies with the fee as
	/// message.
	pub fn verify(&self) -> Result<(), secp::Error> {
		let msg = try!(Message::from_slice(
			&kernel_sig_msg(self.fee, self.lock_height),
		));
		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		let sig = try!(Signature::from_der(&secp, &self.excess_sig));
		secp.verify_from_commit(&msg, &sig, &self.excess)
	}
}

/// A transaction
#[derive(Debug, Clone)]
pub struct Transaction {
	/// Set of inputs spent by the transaction.
	pub inputs: Vec<Input>,
	/// Set of outputs the transaction produces.
	pub outputs: Vec<Output>,
	/// Fee paid by the transaction.
	pub fee: u64,
	/// Transaction is not valid before this block height.
	/// It is invalid for this to be less than the lock_height of any UTXO being spent.
	pub lock_height: u64,
	/// The signature proving the excess is a valid public key, which signs
	/// the transaction fee.
	pub excess_sig: Vec<u8>,
}

/// Implementation of Writeable for a fully blinded transaction, defines how to
/// write the transaction as binary.
impl Writeable for Transaction {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u64, self.fee],
			[write_u64, self.lock_height],
			[write_bytes, &self.excess_sig],
			[write_u64, self.inputs.len() as u64],
			[write_u64, self.outputs.len() as u64]
		);

		// Consensus rule that everything is sorted in lexicographical order on the wire.
		let mut inputs = self.inputs.clone();
		let mut outputs = self.outputs.clone();

		try!(inputs.write_sorted(writer));
		try!(outputs.write_sorted(writer));

		Ok(())
	}
}

/// Implementation of Readable for a transaction, defines how to read a full
/// transaction from a binary stream.
impl Readable for Transaction {
	fn read(reader: &mut Reader) -> Result<Transaction, ser::Error> {
		let (fee, lock_height, excess_sig, input_len, output_len) =
			ser_multiread!(reader, read_u64, read_u64, read_vec, read_u64, read_u64);

		let inputs = read_and_verify_sorted(reader, input_len)?;
		let outputs = read_and_verify_sorted(reader, output_len)?;

		Ok(Transaction {
			fee: fee,
			lock_height: lock_height,
			excess_sig: excess_sig,
			inputs: inputs,
			outputs: outputs,
			..Default::default()
		})
	}
}

impl Committed for Transaction {
	fn inputs_committed(&self) -> &Vec<Input> {
		&self.inputs
	}
	fn outputs_committed(&self) -> &Vec<Output> {
		&self.outputs
	}
	fn overage(&self) -> i64 {
		(self.fee as i64)
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
			fee: 0,
			lock_height: 0,
			excess_sig: vec![],
			inputs: vec![],
			outputs: vec![],
		}
	}

	/// Creates a new transaction initialized with
	/// the provided inputs, outputs, fee and lock_height.
	pub fn new(
		inputs: Vec<Input>,
		outputs: Vec<Output>,
		fee: u64,
		lock_height: u64,
	) -> Transaction {
		Transaction {
			fee: fee,
			lock_height: lock_height,
			excess_sig: vec![],
			inputs: inputs,
			outputs: outputs,
		}
	}

	/// Builds a new transaction with the provided inputs added. Existing
	/// inputs, if any, are kept intact.
	pub fn with_input(self, input: Input) -> Transaction {
		let mut new_ins = self.inputs;
		new_ins.push(input);
		Transaction {
			inputs: new_ins,
			..self
		}
	}

	/// Builds a new transaction with the provided output added. Existing
	/// outputs, if any, are kept intact.
	pub fn with_output(self, output: Output) -> Transaction {
		let mut new_outs = self.outputs;
		new_outs.push(output);
		Transaction {
			outputs: new_outs,
			..self
		}
	}

	/// Builds a new transaction with the provided fee.
	pub fn with_fee(self, fee: u64) -> Transaction {
		Transaction { fee: fee, ..self }
	}

	/// Builds a new transaction with the provided lock_height.
	pub fn with_lock_height(self, lock_height: u64) -> Transaction {
		Transaction {
			lock_height: lock_height,
			..self
		}
	}

	/// The verification for a MimbleWimble transaction involves getting the
	/// excess of summing all commitments and using it as a public key
	/// to verify the embedded signature. The rational is that if the values
	/// sum to zero as they should in r.G + v.H then only k.G the excess
	/// of the sum of r.G should be left. And r.G is the definition of a
	/// public key generated using r as a private key.
	pub fn verify_sig(&self) -> Result<Commitment, secp::Error> {
		let rsum = self.sum_commitments()?;

		let msg = Message::from_slice(&kernel_sig_msg(self.fee, self.lock_height))?;

		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		let sig = Signature::from_der(&secp, &self.excess_sig)?;

		// pretend the sum is a public key (which it is, being of the form r.G) and
		// verify the transaction sig with it
		//
		// we originally converted the commitment to a key_id here (commitment to zero)
		// and then passed the key_id to secp.verify()
		// the secp api no longer allows us to do this so we have wrapped the complexity
		// of generating a public key from a commitment behind verify_from_commit
		secp.verify_from_commit(&msg, &sig, &rsum)?;

		Ok(rsum)
	}

	/// Builds a transaction kernel
	pub fn build_kernel(&self, excess: Commitment) -> TxKernel {
		TxKernel {
			features: DEFAULT_KERNEL,
			excess: excess,
			excess_sig: self.excess_sig.clone(),
			fee: self.fee,
			lock_height: self.lock_height,
		}
	}

	/// Validates all relevant parts of a fully built transaction. Checks the
	/// excess value against the signature as well as range proofs for each
	/// output.
	pub fn validate(&self) -> Result<Commitment, Error> {
		if self.fee & 1 != 0 {
			return Err(Error::OddFee);
		}
		if self.inputs.len() > consensus::MAX_BLOCK_INPUTS {
			return Err(Error::TooManyInputs);
		}
		for out in &self.outputs {
			out.verify_proof()?;
		}
		let excess = self.verify_sig()?;
		Ok(excess)
	}
}

/// A transaction input, mostly a reference to an output being spent by the
/// transaction.
#[derive(Debug, Copy, Clone)]
pub struct Input(pub Commitment);

hashable_ord!(Input);

/// Implementation of Writeable for a transaction Input, defines how to write
/// an Input as binary.
impl Writeable for Input {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_fixed_bytes(&self.0)
	}
}

/// Implementation of Readable for a transaction Input, defines how to read
/// an Input from a binary stream.
impl Readable for Input {
	fn read(reader: &mut Reader) -> Result<Input, ser::Error> {
		Ok(Input(Commitment::read(reader)?))
	}
}

/// The input for a transaction, which spends a pre-existing output. The input
/// commitment is a reproduction of the commitment of the output it's spending.
impl Input {
	/// Extracts the referenced commitment from a transaction output
	pub fn commitment(&self) -> Commitment {
		self.0
	}
}

bitflags! {
	/// Options for block validation
	#[derive(Serialize, Deserialize)]
	pub flags OutputFeatures: u8 {
		/// No flags
		const DEFAULT_OUTPUT = 0b00000000,
		/// Output is a coinbase output, must not be spent until maturity
		const COINBASE_OUTPUT = 0b00000001,
	}
}

/// Definition of the switch commitment hash
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwitchCommitHash {
	/// simple hash
	pub hash: [u8; SWITCH_COMMIT_HASH_SIZE],
}

/// Implementation of Writeable for a switch commitment hash
impl Writeable for SwitchCommitHash {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_fixed_bytes(&self.hash)?;
		Ok(())
	}
}

/// Implementation of Readable for a switch commitment hash
/// an Output from a binary stream.
impl Readable for SwitchCommitHash {
	fn read(reader: &mut Reader) -> Result<SwitchCommitHash, ser::Error> {
		let a = try!(reader.read_fixed_bytes(SWITCH_COMMIT_HASH_SIZE));
		let mut c = [0; SWITCH_COMMIT_HASH_SIZE];
		for i in 0..SWITCH_COMMIT_HASH_SIZE {
			c[i] = a[i];
		}
		Ok(SwitchCommitHash { hash: c })
	}
}
// As Ref for AsFixedBytes
impl AsRef<[u8]> for SwitchCommitHash {
	fn as_ref(&self) -> &[u8] {
		&self.hash
	}
}

impl SwitchCommitHash {
	/// Builds a switch commitment hash from a switch commit using blake2
	pub fn from_switch_commit(switch_commit: Commitment) -> SwitchCommitHash {
		let switch_commit_hash = blake2b(SWITCH_COMMIT_HASH_SIZE, &[], &switch_commit.0);
		let switch_commit_hash = switch_commit_hash.as_bytes();
		let mut h = [0; SWITCH_COMMIT_HASH_SIZE];
		for i in 0..SWITCH_COMMIT_HASH_SIZE {
			h[i] = switch_commit_hash[i];
		}
		SwitchCommitHash { hash: h }
	}
}

/// Output for a transaction, defining the new ownership of coins that are being
/// transferred. The commitment is a blinded value for the output while the
/// range proof guarantees the commitment includes a positive value without
/// overflow and the ownership of the private key. The switch commitment hash
/// provides future-proofing against quantum-based attacks, as well as provides
/// wallet implementations with a way to identify their outputs for wallet
/// reconstruction
///
/// The hash of an output only covers its features, lock_height, commitment,
/// and switch commitment. The range proof is expected to have its own hash
/// and is stored and committed to separately.
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct Output {
	/// Options for an output's structure or use
	pub features: OutputFeatures,
	/// The homomorphic commitment representing the output's amount
	pub commit: Commitment,
	/// The switch commitment hash, a 160 bit length blake2 hash of blind*J
	pub switch_commit_hash: SwitchCommitHash,
	/// A proof that the commitment is in the right range
	pub proof: RangeProof,
}

hashable_ord!(Output);

/// Implementation of Writeable for a transaction Output, defines how to write
/// an Output as binary.
impl Writeable for Output {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(self.features.bits())?;
		writer.write_fixed_bytes(&self.commit)?;
		writer.write_fixed_bytes(&self.switch_commit_hash)?;

		// The hash of an output doesn't include the range proof
		if writer.serialization_mode() == ser::SerializationMode::Full {
			writer.write_bytes(&self.proof)?
		}
		Ok(())
	}
}

/// Implementation of Readable for a transaction Output, defines how to read
/// an Output from a binary stream.
impl Readable for Output {
	fn read(reader: &mut Reader) -> Result<Output, ser::Error> {
		let features = OutputFeatures::from_bits(reader.read_u8()?).ok_or(
			ser::Error::CorruptedData,
		)?;

		Ok(Output {
			features: features,
			commit: Commitment::read(reader)?,
			switch_commit_hash: SwitchCommitHash::read(reader)?,
			proof: RangeProof::read(reader)?,
		})
	}
}

impl Output {
	/// Commitment for the output
	pub fn commitment(&self) -> Commitment {
		self.commit
	}

	/// Switch commitment hash for the output
	pub fn switch_commit_hash(&self) -> SwitchCommitHash {
		self.switch_commit_hash
	}

	/// Range proof for the output
	pub fn proof(&self) -> RangeProof {
		self.proof
	}

	/// Validates the range proof using the commitment
	pub fn verify_proof(&self) -> Result<(), secp::Error> {
		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		secp.verify_range_proof(self.commit, self.proof).map(|_| ())
	}

	/// Given the original blinding factor we can recover the
	/// value from the range proof and the commitment
	pub fn recover_value(&self, keychain: &Keychain, key_id: &Identifier) -> Option<u64> {
		match keychain.rewind_range_proof(key_id, self.commit, self.proof) {
			Ok(proof_info) => {
				if proof_info.success {
					Some(proof_info.value)
				} else {
					None
				}
			}
			Err(_) => None,
		}
	}
}

/// Wrapper to Output commitments to provide the Summable trait.
#[derive(Clone, Debug)]
pub struct SumCommit {
	/// Output commitment
	pub commit: Commitment,
}

/// Outputs get summed through their commitments.
impl Summable for SumCommit {
	type Sum = SumCommit;

	fn sum(&self) -> SumCommit {
		SumCommit { commit: self.commit.clone() }
	}

	fn sum_len() -> usize {
		secp::constants::PEDERSEN_COMMITMENT_SIZE
	}
}

impl Writeable for SumCommit {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.commit.write(writer)?;
		Ok(())
	}
}

impl Readable for SumCommit {
	fn read(reader: &mut Reader) -> Result<SumCommit, ser::Error> {
		let commit = Commitment::read(reader)?;

		Ok(SumCommit { commit: commit })
	}
}

impl ops::Add for SumCommit {
	type Output = SumCommit;

	fn add(self, other: SumCommit) -> SumCommit {
		let secp = static_secp_instance();
		let sum = match secp.lock().unwrap().commit_sum(
			vec![
				self.commit.clone(),
				other.commit.clone(),
			],
			vec![],
		) {
			Ok(s) => s,
			Err(_) => Commitment::from_vec(vec![1; 33]),
		};
		SumCommit { commit: sum }
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use keychain::Keychain;
	use util::secp;

	#[test]
	fn test_kernel_ser_deser() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let commit = keychain.commit(5, &key_id).unwrap();

		// just some bytes for testing ser/deser
		let sig = vec![1, 0, 0, 0, 0, 0, 0, 1];

		let kernel = TxKernel {
			features: DEFAULT_KERNEL,
			lock_height: 0,
			excess: commit,
			excess_sig: sig.clone(),
			fee: 10,
		};

		let mut vec = vec![];
		ser::serialize(&mut vec, &kernel).expect("serialized failed");
		let kernel2: TxKernel = ser::deserialize(&mut &vec[..]).unwrap();
		assert_eq!(kernel2.features, DEFAULT_KERNEL);
		assert_eq!(kernel2.lock_height, 0);
		assert_eq!(kernel2.excess, commit);
		assert_eq!(kernel2.excess_sig, sig.clone());
		assert_eq!(kernel2.fee, 10);

		// now check a kernel with lock_height serializes/deserializes correctly
		let kernel = TxKernel {
			features: DEFAULT_KERNEL,
			lock_height: 100,
			excess: commit,
			excess_sig: sig.clone(),
			fee: 10,
		};

		let mut vec = vec![];
		ser::serialize(&mut vec, &kernel).expect("serialized failed");
		let kernel2: TxKernel = ser::deserialize(&mut &vec[..]).unwrap();
		assert_eq!(kernel2.features, DEFAULT_KERNEL);
		assert_eq!(kernel2.lock_height, 100);
		assert_eq!(kernel2.excess, commit);
		assert_eq!(kernel2.excess_sig, sig.clone());
		assert_eq!(kernel2.fee, 10);
	}

	#[test]
	fn test_output_ser_deser() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let commit = keychain.commit(5, &key_id).unwrap();
		let switch_commit = keychain.switch_commit(&key_id).unwrap();
		let switch_commit_hash = SwitchCommitHash::from_switch_commit(switch_commit);
		let msg = secp::pedersen::ProofMessage::empty();
		let proof = keychain.range_proof(5, &key_id, commit, msg).unwrap();

		let out = Output {
			features: DEFAULT_OUTPUT,
			commit: commit,
			switch_commit_hash: switch_commit_hash,
			proof: proof,
		};

		let mut vec = vec![];
		ser::serialize(&mut vec, &out).expect("serialized failed");
		let dout: Output = ser::deserialize(&mut &vec[..]).unwrap();

		assert_eq!(dout.features, DEFAULT_OUTPUT);
		assert_eq!(dout.commit, out.commit);
		assert_eq!(dout.proof, out.proof);
	}

	#[test]
	fn test_output_value_recovery() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();

		let commit = keychain.commit(1003, &key_id).unwrap();
		let switch_commit = keychain.switch_commit(&key_id).unwrap();
		let switch_commit_hash = SwitchCommitHash::from_switch_commit(switch_commit);
		let msg = secp::pedersen::ProofMessage::empty();
		let proof = keychain.range_proof(1003, &key_id, commit, msg).unwrap();

		let output = Output {
			features: DEFAULT_OUTPUT,
			commit: commit,
			switch_commit_hash: switch_commit_hash,
			proof: proof,
		};

		// check we can successfully recover the value with the original blinding factor
		let recovered_value = output.recover_value(&keychain, &key_id).unwrap();
		assert_eq!(recovered_value, 1003);

		// check we cannot recover the value without the original blinding factor
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let not_recoverable = output.recover_value(&keychain, &key_id2);
		match not_recoverable {
			Some(_) => panic!("expected value to be None here"),
			None => {}
		}
	}

	#[test]
	fn commit_consistency() {
		let keychain = Keychain::from_seed(&[0; 32]).unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();

		let commit = keychain.commit(1003, &key_id).unwrap();
		let switch_commit = keychain.switch_commit(&key_id).unwrap();
		println!("Switch commit: {:?}", switch_commit);
		println!("commit: {:?}", commit);
		let key_id = keychain.derive_key_id(1).unwrap();

		let switch_commit_2 = keychain.switch_commit(&key_id).unwrap();
		let commit_2 = keychain.commit(1003, &key_id).unwrap();
		println!("Switch commit 2: {:?}", switch_commit_2);
		println!("commit2 : {:?}", commit_2);

		assert!(commit == commit_2);
		assert!(switch_commit == switch_commit_2);
	}
}
