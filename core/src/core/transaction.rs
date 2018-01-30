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

//! Transactions
use blake2::blake2b::blake2b;
use util::secp::{self, Message, Signature};
use util::{static_secp_instance, kernel_sig_msg};
use util::secp::pedersen::{Commitment, RangeProof};
use std::cmp::min;
use std::cmp::Ordering;
use std::ops;

use consensus;
use consensus::VerifySortOrder;
use core::Committed;
use core::hash::{Hash, Hashed, ZERO_HASH};
use core::pmmr::Summable;
use keychain::{Identifier, Keychain};
use ser::{self, read_and_verify_sorted, Readable, Reader, Writeable, WriteableSorted, Writer};
use util;

/// The size of the blake2 hash of a switch commitment (256 bits)
pub const SWITCH_COMMIT_HASH_SIZE: usize = 32;

/// The size of the secret key used in to generate blake2 switch commitment hash (256 bits)
pub const SWITCH_COMMIT_KEY_SIZE: usize = 32;

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
	/// Underlying Secp256k1 error (signature validation or invalid public key typically)
	Secp(secp::Error),
	/// Restrict number of incoming inputs
	TooManyInputs,
	/// Underlying consensus error (currently for sort order)
	ConsensusError(consensus::Error),
	/// Error originating from an invalid lock-height
	LockHeight(u64),
	/// Error originating from an invalid switch commitment (coinbase lock_height related)
	SwitchCommitment,
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<consensus::Error> for Error {
	fn from(e: consensus::Error) -> Error {
		Error::ConsensusError(e)
	}
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
	pub excess_sig: Signature,
}

hashable_ord!(TxKernel);

impl Writeable for TxKernel {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u8, self.features.bits()],
			[write_u64, self.fee],
			[write_u64, self.lock_height],
			[write_fixed_bytes, &self.excess]
		);
		self.excess_sig.write(writer)?;
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
			excess_sig: Signature::read(reader)?,
		})
	}
}

impl TxKernel {
	/// Verify the transaction proof validity. Entails handling the commitment
	/// as a public key and checking the signature verifies with the fee as
	/// message.
	pub fn verify(&self) -> Result<(), secp::Error> {
		let msg = Message::from_slice(&kernel_sig_msg(self.fee, self.lock_height))?;
		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		let sig = &self.excess_sig;
		let valid = Keychain::aggsig_verify_single_from_commit(&secp, &sig, &msg, &self.excess);
		if !valid {
			return Err(secp::Error::IncorrectSignature);
		}
		Ok(())
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
	/// Transaction is not valid before this chain height.
	pub lock_height: u64,
	/// The signature proving the excess is a valid public key, which signs
	/// the transaction fee.
	pub excess_sig: Signature,
}

/// Implementation of Writeable for a fully blinded transaction, defines how to
/// write the transaction as binary.
impl Writeable for Transaction {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u64, self.fee],
			[write_u64, self.lock_height]
		);
		self.excess_sig.write(writer)?;
		ser_multiwrite!(
			writer,
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
		let (fee, lock_height) =
			ser_multiread!(reader, read_u64, read_u64);

		let excess_sig = Signature::read(reader)?;

		let (input_len, output_len) =
			ser_multiread!(reader, read_u64, read_u64);

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
			excess_sig: Signature::from_raw_data(&[0;64]).unwrap(),
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
			excess_sig: Signature::from_raw_data(&[0;64]).unwrap(),
			inputs: inputs,
			outputs: outputs,
		}
	}

	/// Builds a new transaction with the provided inputs added. Existing
	/// inputs, if any, are kept intact.
	pub fn with_input(self, input: Input) -> Transaction {
		let mut new_ins = self.inputs;
		new_ins.push(input);
		new_ins.sort();
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
		new_outs.sort();
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
		let sig = self.excess_sig;
		// pretend the sum is a public key (which it is, being of the form r.G) and
		// verify the transaction sig with it
		let valid = Keychain::aggsig_verify_single_from_commit(&secp, &sig, &msg, &rsum);
		if !valid {
			return Err(secp::Error::IncorrectSignature);
		}
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
		self.verify_sorted()?;
		for out in &self.outputs {
			out.verify_proof()?;
		}
		let excess = self.verify_sig()?;
		Ok(excess)
	}

	fn verify_sorted(&self) -> Result<(), Error> {
		self.inputs.verify_sort_order()?;
		self.outputs.verify_sort_order()?;
		Ok(())
	}
}

/// A transaction input.
///
/// Primarily a reference to an output being spent by the transaction.
/// But also information required to verify coinbase maturity through
/// the lock_height hashed in the switch_commit_hash.
#[derive(Debug, Clone, Copy)]
pub struct Input{
	/// The features of the output being spent.
	/// We will check maturity for coinbase output.
	pub features: OutputFeatures,
	/// The commit referencing the output being spent.
	pub commit: Commitment,
	/// The hash of the block the output originated from.
	/// Currently we only care about this for coinbase outputs.
	/// TODO - include the merkle proof here once we support these.
	pub out_block: Option<Hash>,
}

hashable_ord!(Input);

/// Implementation of Writeable for a transaction Input, defines how to write
/// an Input as binary.
impl Writeable for Input {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(self.features.bits())?;
		writer.write_fixed_bytes(&self.commit)?;

		if self.features.contains(COINBASE_OUTPUT) {
			writer.write_fixed_bytes(&self.out_block.unwrap_or(ZERO_HASH))?;
		}

		Ok(())
	}
}

/// Implementation of Readable for a transaction Input, defines how to read
/// an Input from a binary stream.
impl Readable for Input {
	fn read(reader: &mut Reader) -> Result<Input, ser::Error> {
		let features = OutputFeatures::from_bits(reader.read_u8()?).ok_or(
			ser::Error::CorruptedData,
		)?;

		let commit = Commitment::read(reader)?;

		let out_block = if features.contains(COINBASE_OUTPUT) {
			Some(Hash::read(reader)?)
		} else {
			None
		};

		Ok(Input::new(
			features,
			commit,
			out_block,
		))
	}
}

/// The input for a transaction, which spends a pre-existing unspent output.
/// The input commitment is a reproduction of the commitment of the output being spent.
/// Input must also provide the original output features and the hash of the block
/// the output originated from.
impl Input {
	/// Build a new input from the data required to identify and verify an output beng spent.
	pub fn new(
		features: OutputFeatures,
		commit: Commitment,
		out_block: Option<Hash>,
	) -> Input {
		Input {
			features,
			commit,
			out_block,
		}
	}

	/// The input commitment which _partially_ identifies the output being spent.
	/// In the presence of a fork we need additional info to uniquely identify the output.
	/// Specifically the block hash (so correctly calculate lock_height for coinbase outputs).
	pub fn commitment(&self) -> Commitment {
		self.commit
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
pub struct SwitchCommitHashKey ([u8; SWITCH_COMMIT_KEY_SIZE]);

impl SwitchCommitHashKey {
	/// We use a zero value key for regular transactions.
	pub fn zero() -> SwitchCommitHashKey {
		SwitchCommitHashKey([0; SWITCH_COMMIT_KEY_SIZE])
	}

	/// Generate a switch commit hash key from the provided keychain and key id.
	pub fn from_keychain(keychain: &Keychain, key_id: &Identifier) -> SwitchCommitHashKey {
		SwitchCommitHashKey(
			keychain.switch_commit_hash_key(key_id)
				.expect("failed to derive switch commit hash key")
		)
	}

	/// Reconstructs a switch commit hash key from a byte slice.
	pub fn from_bytes(bytes: &[u8]) -> SwitchCommitHashKey {
		assert!(bytes.len() == 32, "switch_commit_hash_key requires 32 bytes");

		let mut key = [0; SWITCH_COMMIT_KEY_SIZE];
		for i in 0..min(SWITCH_COMMIT_KEY_SIZE, bytes.len()) {
			key[i] = bytes[i];
		}
		SwitchCommitHashKey(key)
	}
}

/// Definition of the switch commitment hash
#[derive(Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwitchCommitHash([u8; SWITCH_COMMIT_HASH_SIZE]);

/// Implementation of Writeable for a switch commitment hash
impl Writeable for SwitchCommitHash {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_fixed_bytes(&self.0)?;
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
		Ok(SwitchCommitHash(c))
	}
}
// As Ref for AsFixedBytes
impl AsRef<[u8]> for SwitchCommitHash {
	fn as_ref(&self) -> &[u8] {
		&self.0
	}
}

impl ::std::fmt::Debug for SwitchCommitHash {
	fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
		try!(write!(f, "{}(", stringify!(SwitchCommitHash)));
		try!(write!(f, "{}", self.to_hex()));
		write!(f, ")")
	}
}

impl SwitchCommitHash {
	/// Builds a switch commit hash from a switch commit using blake2
	pub fn from_switch_commit(
		switch_commit: Commitment,
		keychain: &Keychain,
		key_id: &Identifier,
	) -> SwitchCommitHash {
		let key = SwitchCommitHashKey::from_keychain(keychain, key_id);
		let switch_commit_hash = blake2b(SWITCH_COMMIT_HASH_SIZE, &key.0, &switch_commit.0);
		let switch_commit_hash_bytes = switch_commit_hash.as_bytes();
		let mut h = [0; SWITCH_COMMIT_HASH_SIZE];
		for i in 0..SWITCH_COMMIT_HASH_SIZE {
			h[i] = switch_commit_hash_bytes[i];
		}
		SwitchCommitHash(h)
	}

	/// Reconstructs a switch commit hash from a byte slice.
	pub fn from_bytes(bytes: &[u8]) -> SwitchCommitHash {
		let mut hash = [0; SWITCH_COMMIT_HASH_SIZE];
		for i in 0..min(SWITCH_COMMIT_HASH_SIZE, bytes.len()) {
			hash[i] = bytes[i];
		}
		SwitchCommitHash(hash)
	}

	/// Hex string representation of a switch commitment hash.
	pub fn to_hex(&self) -> String {
		util::to_hex(self.0.to_vec())
	}

	/// Reconstructs a switch commit hash from a hex string.
	pub fn from_hex(hex: &str) -> Result<SwitchCommitHash, ser::Error> {
		let bytes = util::from_hex(hex.to_string())
			.map_err(|_| ser::Error::HexError(format!("switch_commit_hash from_hex error")))?;
		Ok(SwitchCommitHash::from_bytes(&bytes))
	}

	/// Build an "zero" switch commitment hash
	pub fn zero() -> SwitchCommitHash {
		SwitchCommitHash([0; SWITCH_COMMIT_HASH_SIZE])
	}
}

/// Output for a transaction, defining the new ownership of coins that are being
/// transferred. The commitment is a blinded value for the output while the
/// range proof guarantees the commitment includes a positive value without
/// overflow and the ownership of the private key. The switch commitment hash
/// provides future-proofing against quantum-based attacks, as well as providing
/// wallet implementations with a way to identify their outputs for wallet
/// reconstruction.
///
/// The hash of an output only covers its features, commitment,
/// and switch commitment. The range proof is expected to have its own hash
/// and is stored and committed to separately.
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct Output {
	/// Options for an output's structure or use
	pub features: OutputFeatures,
	/// The homomorphic commitment representing the output amount
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

/// An output_identifier can be build from either an input _or_ and output and
/// contains everything we need to uniquely identify an output being spent.
/// Needed because it is not sufficient to pass a commitment around.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OutputIdentifier {
	/// Output features (coinbase vs. regular transaction output)
	/// We need to include this when hashing to ensure coinbase maturity can be enforced.
	pub features: OutputFeatures,
	/// Output commitment
	pub commit: Commitment,
}

impl OutputIdentifier {
	/// Build a new output_identifier.
	pub fn new(features: OutputFeatures, commit: &Commitment) -> OutputIdentifier {
		OutputIdentifier {
			features: features.clone(),
			commit: commit.clone(),
		}
	}

	/// Build an output_identifier from an existing output.
	pub fn from_output(output: &Output) -> OutputIdentifier {
		OutputIdentifier {
			features: output.features,
			commit: output.commit,
		}
	}

	/// Build an output_identifier from an existing input.
	pub fn from_input(input: &Input) -> OutputIdentifier {
		OutputIdentifier {
			features: input.features,
			commit: input.commit,
		}
	}

	/// convert an output_identifier to hex string format.
	pub fn to_hex(&self) -> String {
		format!(
			"{:b}{}",
			self.features.bits(),
			util::to_hex(self.commit.0.to_vec()),
		)
	}

	/// Convert an output_indentifier to a sum_commit representation
	/// so we can use it to query the the output MMR
	pub fn as_sum_commit(&self) -> SumCommit {
		SumCommit {
			features: self.features,
			commit: self.commit,
			switch_commit_hash: SwitchCommitHash::zero(),
		}
	}

	/// Convert a sum_commit back to an output_identifier.
	pub fn from_sum_commit(sum_commit: &SumCommit) -> OutputIdentifier {
		OutputIdentifier::new(sum_commit.features, &sum_commit.commit)
	}
}

impl Writeable for OutputIdentifier {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(self.features.bits())?;
		self.commit.write(writer)?;
		Ok(())
	}
}

impl Readable for OutputIdentifier {
	fn read(reader: &mut Reader) -> Result<OutputIdentifier, ser::Error> {
		let features = OutputFeatures::from_bits(reader.read_u8()?).ok_or(
			ser::Error::CorruptedData,
		)?;
		Ok(OutputIdentifier {
			commit: Commitment::read(reader)?,
			features: features,
		})
	}
}

/// Wrapper to Output commitments to provide the Summable trait.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SumCommit {
	/// Output features (coinbase vs. regular transaction output)
	/// We need to include this when hashing to ensure coinbase maturity can be enforced.
	pub features: OutputFeatures,
	/// Output commitment
	pub commit: Commitment,
	/// The corresponding switch commit hash
	pub switch_commit_hash: SwitchCommitHash,
}

impl SumCommit {
	/// Build a new sum_commit.
	pub fn new(
		features: OutputFeatures,
		commit: &Commitment,
		switch_commit_hash: &SwitchCommitHash,
	) -> SumCommit {
		SumCommit {
			features: features.clone(),
			commit: commit.clone(),
			switch_commit_hash: switch_commit_hash.clone(),
		}
	}

	/// Build a new sum_commit from an existing output.
	pub fn from_output(output: &Output) -> SumCommit {
		SumCommit {
			features: output.features,
			commit: output.commit,
			switch_commit_hash: output.switch_commit_hash,
		}
	}

	/// Build a new sum_commit from an existing input.
	pub fn from_input(input: &Input) -> SumCommit {
		SumCommit {
			features: input.features,
			commit: input.commit,
			switch_commit_hash: SwitchCommitHash::zero(),
		}
	}

	/// Hex string representation of a sum_commit.
	pub fn to_hex(&self) -> String {
		format!(
			"{:b}{}{}",
			self.features.bits(),
			util::to_hex(self.commit.0.to_vec()),
			self.switch_commit_hash.to_hex(),
		)
	}
}

/// Outputs get summed through their commitments.
impl Summable for SumCommit {
	type Sum = SumCommit;

	fn sum(&self) -> SumCommit {
		SumCommit {
			commit: self.commit.clone(),
			features: self.features.clone(),
			switch_commit_hash: self.switch_commit_hash.clone(),
		}
	}

	fn sum_len() -> usize {
		secp::constants::PEDERSEN_COMMITMENT_SIZE + SWITCH_COMMIT_HASH_SIZE + 1
	}
}

impl Writeable for SumCommit {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(self.features.bits())?;
		self.commit.write(writer)?;
		if writer.serialization_mode() == ser::SerializationMode::Full {
			self.switch_commit_hash.write(writer)?;
		}
		Ok(())
	}
}

impl Readable for SumCommit {
	fn read(reader: &mut Reader) -> Result<SumCommit, ser::Error> {
		let features = OutputFeatures::from_bits(reader.read_u8()?).ok_or(
			ser::Error::CorruptedData,
		)?;
		Ok(SumCommit {
			features: features,
			commit: Commitment::read(reader)?,
			switch_commit_hash: SwitchCommitHash::read(reader)?,
		})
	}
}

impl ops::Add for SumCommit {
	type Output = SumCommit;

	fn add(self, other: SumCommit) -> SumCommit {
		// Build a new commitment by summing the two commitments.
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

		// Now build a new switch_commit_hash by concatenating the two switch_commit_hash value
		// and hashing the result.
		let mut bytes = self.switch_commit_hash.0.to_vec();
		bytes.extend(other.switch_commit_hash.0.iter().cloned());
		let key = SwitchCommitHashKey::zero();
		let hash = blake2b(SWITCH_COMMIT_HASH_SIZE, &key.0, &bytes);
		let hash = hash.as_bytes();
		let mut h = [0; SWITCH_COMMIT_HASH_SIZE];
		for i in 0..SWITCH_COMMIT_HASH_SIZE {
			h[i] = hash[i];
		}
		let switch_commit_hash_sum = SwitchCommitHash(h);

		SumCommit {
			features: self.features | other.features,
			commit: sum,
			switch_commit_hash: switch_commit_hash_sum,
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use core::id::{ShortId, ShortIdentifiable};
	use keychain::Keychain;
	use util::secp;

	#[test]
	fn test_kernel_ser_deser() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let commit = keychain.commit(5, &key_id).unwrap();

		// just some bytes for testing ser/deser
		let sig = secp::Signature::from_raw_data(&[0;64]).unwrap();

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
		let switch_commit_hash = SwitchCommitHash::from_switch_commit(
			switch_commit,
			&keychain,
			&key_id,
		);
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
		let switch_commit_hash = SwitchCommitHash::from_switch_commit(
			switch_commit,
			&keychain,
			&key_id,
		);
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

	#[test]
	fn input_short_id() {
		let keychain = Keychain::from_seed(&[0; 32]).unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let commit = keychain.commit(5, &key_id).unwrap();

		let input = Input {
			features: DEFAULT_OUTPUT,
			commit: commit,
			out_block: None,
		};

		let block_hash = Hash::from_hex(
			"3a42e66e46dd7633b57d1f921780a1ac715e6b93c19ee52ab714178eb3a9f673",
		).unwrap();

		let short_id = input.short_id(&block_hash);
		assert_eq!(short_id, ShortId::from_hex("3e1262905b7a").unwrap());

		// now generate the short_id for a *very* similar output (single feature flag different)
		// and check it generates a different short_id
		let input = Input {
			features: COINBASE_OUTPUT,
			commit: commit,
			out_block: None,
		};

		let block_hash = Hash::from_hex(
			"3a42e66e46dd7633b57d1f921780a1ac715e6b93c19ee52ab714178eb3a9f673",
		).unwrap();

		let short_id = input.short_id(&block_hash);
		assert_eq!(short_id, ShortId::from_hex("90653c1c870a").unwrap());
	}
}
