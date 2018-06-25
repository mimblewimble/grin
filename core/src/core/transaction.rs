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

use std::cmp::Ordering;
use std::cmp::max;
use std::collections::HashSet;
use std::io::Cursor;
use std::{error, fmt};

use util::secp::pedersen::{Commitment, ProofMessage, RangeProof};
use util::secp::{self, Message, Signature};
use util::{kernel_sig_msg, static_secp_instance};

use consensus::{self, VerifySortOrder};
use core::hash::{Hash, Hashed, ZERO_HASH};
use core::merkle_proof::MerkleProof;
use core::{committed, Committed};
use keychain::{self, BlindingFactor};
use ser::{self, read_and_verify_sorted, ser_vec, PMMRable, Readable, Reader, Writeable,
          WriteableSorted, Writer};
use util;

bitflags! {
	/// Options for a kernel's structure or use
	#[derive(Serialize, Deserialize)]
	pub struct KernelFeatures: u8 {
		/// No flags
		const DEFAULT_KERNEL = 0b00000000;
		/// Kernel matching a coinbase output
		const COINBASE_KERNEL = 0b00000001;
	}
}

/// Errors thrown by Block validation
#[derive(Clone, Eq, Debug, PartialEq)]
pub enum Error {
	/// Underlying Secp256k1 error (signature validation or invalid public key
	/// typically)
	Secp(secp::Error),
	/// Underlying keychain related error
	Keychain(keychain::Error),
	/// The sum of output minus input commitments does not
	/// match the sum of kernel commitments
	KernelSumMismatch,
	/// Restrict number of incoming inputs
	TooManyInputs,
	/// Underlying consensus error (currently for sort order)
	ConsensusError(consensus::Error),
	/// Error originating from an invalid lock-height
	LockHeight(u64),
	/// Range proof validation error
	RangeProof,
	/// Error originating from an invalid Merkle proof
	MerkleProof,
	/// Returns if the value hidden within the a RangeProof message isn't
	/// repeated 3 times, indicating it's incorrect
	InvalidProofMessage,
	/// Error when verifying kernel sums via committed trait.
	Committed(committed::Error),
	/// Error when sums do not verify correctly during tx aggregation.
	/// Likely a "double spend" across two unconfirmed txs.
	AggregationError,
	/// Validation error relating to cut-through (tx is spending its own
	/// output).
	CutThrough,
}

impl error::Error for Error {
	fn description(&self) -> &str {
		match *self {
			_ => "some kind of keychain error",
		}
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			_ => write!(f, "some kind of keychain error"),
		}
	}
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

impl From<keychain::Error> for Error {
	fn from(e: keychain::Error) -> Error {
		Error::Keychain(e)
	}
}

impl From<committed::Error> for Error {
	fn from(e: committed::Error) -> Error {
		Error::Committed(e)
	}
}

/// A proof that a transaction sums to zero. Includes both the transaction's
/// Pedersen commitment and the signature, that guarantees that the commitments
/// amount to zero.
/// The signature signs the fee and the lock_height, which are retained for
/// signature validation.
#[derive(Serialize, Deserialize, Debug, Clone)]
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

/// TODO - no clean way to bridge core::hash::Hash and std::hash::Hash
/// implementations?
impl ::std::hash::Hash for TxKernel {
	fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
		let mut vec = Vec::new();
		ser::serialize(&mut vec, &self).expect("serialization failed");
		::std::hash::Hash::hash(&vec, state);
	}
}

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
		let features =
			KernelFeatures::from_bits(reader.read_u8()?).ok_or(ser::Error::CorruptedData)?;
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
	/// Return the excess commitment for this tx_kernel.
	pub fn excess(&self) -> Commitment {
		self.excess
	}

	/// Verify the transaction proof validity. Entails handling the commitment
	/// as a public key and checking the signature verifies with the fee as
	/// message.
	pub fn verify(&self) -> Result<(), secp::Error> {
		let msg = Message::from_slice(&kernel_sig_msg(self.fee, self.lock_height))?;
		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		let sig = &self.excess_sig;
		// Verify aggsig directly in libsecp
		let pubkey = &self.excess.to_pubkey(&secp)?;
		if !secp::aggsig::verify_single(&secp, &sig, &msg, None, &pubkey, false) {
			return Err(secp::Error::IncorrectSignature);
		}
		Ok(())
	}

	/// Build an empty tx kernel with zero values.
	pub fn empty() -> TxKernel {
		TxKernel {
			features: KernelFeatures::DEFAULT_KERNEL,
			fee: 0,
			lock_height: 0,
			excess: Commitment::from_vec(vec![0; 33]),
			excess_sig: Signature::from_raw_data(&[0; 64]).unwrap(),
		}
	}

	/// Builds a new tx kernel with the provided fee.
	pub fn with_fee(self, fee: u64) -> TxKernel {
		TxKernel { fee: fee, ..self }
	}

	/// Builds a new tx kernel with the provided lock_height.
	pub fn with_lock_height(self, lock_height: u64) -> TxKernel {
		TxKernel {
			lock_height: lock_height,
			..self
		}
	}
}

impl PMMRable for TxKernel {
	fn len() -> usize {
		17 + // features plus fee and lock_height
			secp::constants::PEDERSEN_COMMITMENT_SIZE + secp::constants::AGG_SIGNATURE_SIZE
	}
}

/// A transaction
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
	/// List of inputs spent by the transaction.
	pub inputs: Vec<Input>,
	/// List of outputs the transaction produces.
	pub outputs: Vec<Output>,
	/// List of kernels that make up this transaction (usually a single kernel).
	pub kernels: Vec<TxKernel>,
	/// The kernel "offset" k2
	/// excess is k1G after splitting the key k = k1 + k2
	pub offset: BlindingFactor,
}

/// PartialEq
impl PartialEq for Transaction {
	fn eq(&self, tx: &Transaction) -> bool {
		self.inputs == tx.inputs && self.outputs == tx.outputs && self.kernels == tx.kernels
			&& self.offset == tx.offset
	}
}

/// Implementation of Writeable for a fully blinded transaction, defines how to
/// write the transaction as binary.
impl Writeable for Transaction {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.offset.write(writer)?;
		ser_multiwrite!(
			writer,
			[write_u64, self.inputs.len() as u64],
			[write_u64, self.outputs.len() as u64],
			[write_u64, self.kernels.len() as u64]
		);

		// Consensus rule that everything is sorted in lexicographical order on the
		// wire.
		let mut inputs = self.inputs.clone();
		let mut outputs = self.outputs.clone();
		let mut kernels = self.kernels.clone();

		inputs.write_sorted(writer)?;
		outputs.write_sorted(writer)?;
		kernels.write_sorted(writer)?;

		Ok(())
	}
}

/// Implementation of Readable for a transaction, defines how to read a full
/// transaction from a binary stream.
impl Readable for Transaction {
	fn read(reader: &mut Reader) -> Result<Transaction, ser::Error> {
		let offset = BlindingFactor::read(reader)?;

		let (input_len, output_len, kernel_len) =
			ser_multiread!(reader, read_u64, read_u64, read_u64);

		if input_len > consensus::MAX_TX_INPUTS || output_len > consensus::MAX_TX_OUTPUTS
			|| kernel_len > consensus::MAX_TX_KERNELS
		{
			return Err(ser::Error::CorruptedData);
		}

		let inputs = read_and_verify_sorted(reader, input_len)?;
		let outputs = read_and_verify_sorted(reader, output_len)?;
		let kernels = read_and_verify_sorted(reader, kernel_len)?;

		Ok(Transaction {
			offset,
			inputs,
			outputs,
			kernels,
		})
	}
}

impl Committed for Transaction {
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

impl Default for Transaction {
	fn default() -> Transaction {
		Transaction::empty()
	}
}

impl Transaction {
	/// Creates a new empty transaction (no inputs or outputs, zero fee).
	pub fn empty() -> Transaction {
		Transaction {
			offset: BlindingFactor::zero(),
			inputs: vec![],
			outputs: vec![],
			kernels: vec![],
		}
	}

	/// Creates a new transaction initialized with
	/// the provided inputs, outputs, kernels
	pub fn new(inputs: Vec<Input>, outputs: Vec<Output>, kernels: Vec<TxKernel>) -> Transaction {
		Transaction {
			offset: BlindingFactor::zero(),
			inputs: inputs,
			outputs: outputs,
			kernels: kernels,
		}
	}

	/// Creates a new transaction using this transaction as a template
	/// and with the specified offset.
	pub fn with_offset(self, offset: BlindingFactor) -> Transaction {
		Transaction {
			offset: offset,
			..self
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

	/// Total fee for a transaction is the sum of fees of all kernels.
	pub fn fee(&self) -> u64 {
		self.kernels.iter().fold(0, |acc, ref x| acc + x.fee)
	}

	fn overage(&self) -> i64 {
		self.fee() as i64
	}

	/// Lock height of a transaction is the max lock height of the kernels.
	pub fn lock_height(&self) -> u64 {
		self.kernels
			.iter()
			.fold(0, |acc, ref x| max(acc, x.lock_height))
	}

	/// Verify the kernel signatures.
	/// Note: this is expensive.
	fn verify_kernel_signatures(&self) -> Result<(), Error> {
		for x in &self.kernels {
			x.verify()?;
		}
		Ok(())
	}

	/// Verify all the output rangeproofs.
	/// Note: this is expensive.
	fn verify_rangeproofs(&self) -> Result<(), Error> {
		for x in &self.outputs {
			x.verify_proof()?;
		}
		Ok(())
	}

	/// Validates all relevant parts of a fully built transaction. Checks the
	/// excess value against the signature as well as range proofs for each
	/// output.
	pub fn validate(&self) -> Result<(), Error> {
		if self.inputs.len() > consensus::MAX_BLOCK_INPUTS {
			return Err(Error::TooManyInputs);
		}
		self.verify_sorted()?;
		self.verify_cut_through()?;
		self.verify_kernel_sums(self.overage(), self.offset)?;
		self.verify_rangeproofs()?;
		self.verify_kernel_signatures()?;

		Ok(())
	}

	/// Calculate transaction weight
	pub fn tx_weight(&self) -> u32 {
		Transaction::weight(
			self.inputs.len(),
			self.outputs.len(),
			self.input_proofs_count(),
		)
	}

	/// Collect input's Merkle proofs
	pub fn input_proofs_count(&self) -> usize {
		self.inputs
			.iter()
			.filter(|i| i.merkle_proof.is_some())
			.count()
	}

	/// Calculate transaction weight from transaction details
	pub fn weight(input_len: usize, output_len: usize, proof_len: usize) -> u32 {
		let mut tx_weight =
			-1 * (input_len as i32) + (4 * output_len as i32) + (proof_len as i32) + 1;
		if tx_weight < 1 {
			tx_weight = 1;
		}
		tx_weight as u32
	}

	// Verify that inputs|outputs|kernels are all sorted in lexicographical order.
	fn verify_sorted(&self) -> Result<(), Error> {
		self.inputs.verify_sort_order()?;
		self.outputs.verify_sort_order()?;
		self.kernels.verify_sort_order()?;
		Ok(())
	}

	// Verify that no input is spending an output from the same block.
	fn verify_cut_through(&self) -> Result<(), Error> {
		for inp in &self.inputs {
			if self.outputs
				.iter()
				.any(|out| out.commitment() == inp.commitment())
			{
				return Err(Error::CutThrough);
			}
		}
		Ok(())
	}
}

/// Aggregate a vec of transactions into a multi-kernel transaction with
/// cut_through
pub fn aggregate(transactions: Vec<Transaction>) -> Result<Transaction, Error> {
	let mut inputs: Vec<Input> = vec![];
	let mut outputs: Vec<Output> = vec![];
	let mut kernels: Vec<TxKernel> = vec![];

	// we will sum these together at the end to give us the overall offset for the
	// transaction
	let mut kernel_offsets: Vec<BlindingFactor> = vec![];

	for mut transaction in transactions {
		// we will sum these later to give a single aggregate offset
		kernel_offsets.push(transaction.offset);

		inputs.append(&mut transaction.inputs);
		outputs.append(&mut transaction.outputs);
		kernels.append(&mut transaction.kernels);
	}

	// now sum the kernel_offsets up to give us an aggregate offset for the
	// transaction
	let total_kernel_offset = {
		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		let mut keys = kernel_offsets
			.into_iter()
			.filter(|x| *x != BlindingFactor::zero())
			.filter_map(|x| x.secret_key(&secp).ok())
			.collect::<Vec<_>>();

		if keys.is_empty() {
			BlindingFactor::zero()
		} else {
			let sum = secp.blind_sum(keys, vec![])?;
			BlindingFactor::from_secret_key(sum)
		}
	};

	let in_set = inputs
		.iter()
		.map(|inp| inp.commitment())
		.collect::<HashSet<_>>();

	let out_set = outputs
		.iter()
		.filter(|out| !out.features.contains(OutputFeatures::COINBASE_OUTPUT))
		.map(|out| out.commitment())
		.collect::<HashSet<_>>();

	let to_cut_through = in_set.intersection(&out_set).collect::<HashSet<_>>();

	let mut new_inputs = inputs
		.into_iter()
		.filter(|inp| !to_cut_through.contains(&inp.commitment()))
		.collect::<Vec<_>>();

	let mut new_outputs = outputs
		.into_iter()
		.filter(|out| !to_cut_through.contains(&out.commitment()))
		.collect::<Vec<_>>();

	// sort them lexicographically
	new_inputs.sort();
	new_outputs.sort();
	kernels.sort();

	let tx = Transaction::new(new_inputs, new_outputs, kernels).with_offset(total_kernel_offset);

	// We need to check sums here as aggregation/cut-through
	// may have created an invalid tx.
	tx.verify_kernel_sums(tx.overage(), tx.offset)?;

	Ok(tx)
}

/// Attempt to deaggregate a multi-kernel transaction based on multiple
/// transactions
pub fn deaggregate(mk_tx: Transaction, txs: Vec<Transaction>) -> Result<Transaction, Error> {
	let mut inputs: Vec<Input> = vec![];
	let mut outputs: Vec<Output> = vec![];
	let mut kernels: Vec<TxKernel> = vec![];

	// we will subtract these at the end to give us the overall offset for the
	// transaction
	let mut kernel_offsets = vec![];

	let tx = aggregate(txs)?;

	for mk_input in mk_tx.inputs {
		if !tx.inputs.contains(&mk_input) && !inputs.contains(&mk_input) {
			inputs.push(mk_input);
		}
	}
	for mk_output in mk_tx.outputs {
		if !tx.outputs.contains(&mk_output) && !outputs.contains(&mk_output) {
			outputs.push(mk_output);
		}
	}
	for mk_kernel in mk_tx.kernels {
		if !tx.kernels.contains(&mk_kernel) && !kernels.contains(&mk_kernel) {
			kernels.push(mk_kernel);
		}
	}

	kernel_offsets.push(tx.offset);

	// now compute the total kernel offset
	let total_kernel_offset = {
		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		let mut positive_key = vec![mk_tx.offset]
			.into_iter()
			.filter(|x| *x != BlindingFactor::zero())
			.filter_map(|x| x.secret_key(&secp).ok())
			.collect::<Vec<_>>();
		let mut negative_keys = kernel_offsets
			.into_iter()
			.filter(|x| *x != BlindingFactor::zero())
			.filter_map(|x| x.secret_key(&secp).ok())
			.collect::<Vec<_>>();

		if positive_key.is_empty() && negative_keys.is_empty() {
			BlindingFactor::zero()
		} else {
			let sum = secp.blind_sum(positive_key, negative_keys)?;
			BlindingFactor::from_secret_key(sum)
		}
	};

	// Sorting them lexicographically
	inputs.sort();
	outputs.sort();
	kernels.sort();

	let mut tx = Transaction::new(inputs, outputs, kernels);
	tx.offset = total_kernel_offset;

	Ok(tx)
}

/// A transaction input.
///
/// Primarily a reference to an output being spent by the transaction.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Input {
	/// The features of the output being spent.
	/// We will check maturity for coinbase output.
	pub features: OutputFeatures,
	/// The commit referencing the output being spent.
	pub commit: Commitment,
	/// The hash of the block the output originated from.
	/// Currently we only care about this for coinbase outputs.
	pub block_hash: Option<Hash>,
	/// The Merkle Proof that shows the output being spent by this input
	/// existed and was unspent at the time of this block (proof of inclusion
	/// in output_root)
	pub merkle_proof: Option<MerkleProof>,
}

hashable_ord!(Input);

/// TODO - no clean way to bridge core::hash::Hash and std::hash::Hash
/// implementations?
impl ::std::hash::Hash for Input {
	fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
		let mut vec = Vec::new();
		ser::serialize(&mut vec, &self).expect("serialization failed");
		::std::hash::Hash::hash(&vec, state);
	}
}

/// Implementation of Writeable for a transaction Input, defines how to write
/// an Input as binary.
impl Writeable for Input {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(self.features.bits())?;
		self.commit.write(writer)?;

		if writer.serialization_mode() != ser::SerializationMode::Hash {
			if self.features.contains(OutputFeatures::COINBASE_OUTPUT) {
				let block_hash = &self.block_hash.unwrap_or(ZERO_HASH);
				let merkle_proof = self.merkle_proof();

				writer.write_fixed_bytes(block_hash)?;
				merkle_proof.write(writer)?;
			}
		}

		Ok(())
	}
}

/// Implementation of Readable for a transaction Input, defines how to read
/// an Input from a binary stream.
impl Readable for Input {
	fn read(reader: &mut Reader) -> Result<Input, ser::Error> {
		let features =
			OutputFeatures::from_bits(reader.read_u8()?).ok_or(ser::Error::CorruptedData)?;

		let commit = Commitment::read(reader)?;

		if features.contains(OutputFeatures::COINBASE_OUTPUT) {
			let block_hash = Some(Hash::read(reader)?);
			let merkle_proof = Some(MerkleProof::read(reader)?);
			Ok(Input::new(features, commit, block_hash, merkle_proof))
		} else {
			Ok(Input::new(features, commit, None, None))
		}
	}
}

/// The input for a transaction, which spends a pre-existing unspent output.
/// The input commitment is a reproduction of the commitment of the output
/// being spent. Input must also provide the original output features and the
/// hash of the block the output originated from.
impl Input {
	/// Build a new input from the data required to identify and verify an
	/// output being spent.
	pub fn new(
		features: OutputFeatures,
		commit: Commitment,
		block_hash: Option<Hash>,
		merkle_proof: Option<MerkleProof>,
	) -> Input {
		Input {
			features,
			commit,
			block_hash,
			merkle_proof,
		}
	}

	/// The input commitment which _partially_ identifies the output being
	/// spent. In the presence of a fork we need additional info to uniquely
	/// identify the output. Specifically the block hash (to correctly
	/// calculate lock_height for coinbase outputs).
	pub fn commitment(&self) -> Commitment {
		self.commit
	}

	/// Convenience function to return the (optional) block_hash for this input.
	/// Will return the default hash if we do not have one.
	pub fn block_hash(&self) -> Hash {
		self.block_hash.unwrap_or_else(Hash::default)
	}

	/// Convenience function to return the (optional) merkle_proof for this
	/// input. Will return the "empty" Merkle proof if we do not have one.
	/// We currently only care about the Merkle proof for inputs spending
	/// coinbase outputs.
	pub fn merkle_proof(&self) -> MerkleProof {
		let merkle_proof = self.merkle_proof.clone();
		merkle_proof.unwrap_or_else(MerkleProof::empty)
	}
}

bitflags! {
	/// Options for block validation
	#[derive(Serialize, Deserialize)]
	pub struct OutputFeatures: u8 {
		/// No flags
		const DEFAULT_OUTPUT = 0b00000000;
		/// Output is a coinbase output, must not be spent until maturity
		const COINBASE_OUTPUT = 0b00000001;
	}
}

/// Output for a transaction, defining the new ownership of coins that are being
/// transferred. The commitment is a blinded value for the output while the
/// range proof guarantees the commitment includes a positive value without
/// overflow and the ownership of the private key. The switch commitment hash
/// provides future-proofing against quantum-based attacks, as well as providing
/// wallet implementations with a way to identify their outputs for wallet
/// reconstruction.
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct Output {
	/// Options for an output's structure or use
	pub features: OutputFeatures,
	/// The homomorphic commitment representing the output amount
	pub commit: Commitment,
	/// A proof that the commitment is in the right range
	pub proof: RangeProof,
}

hashable_ord!(Output);

/// TODO - no clean way to bridge core::hash::Hash and std::hash::Hash
/// implementations?
impl ::std::hash::Hash for Output {
	fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
		let mut vec = Vec::new();
		ser::serialize(&mut vec, &self).expect("serialization failed");
		::std::hash::Hash::hash(&vec, state);
	}
}

/// Implementation of Writeable for a transaction Output, defines how to write
/// an Output as binary.
impl Writeable for Output {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(self.features.bits())?;
		self.commit.write(writer)?;
		// The hash of an output doesn't include the range proof, which
		// is commit to separately
		if writer.serialization_mode() != ser::SerializationMode::Hash {
			writer.write_bytes(&self.proof)?
		}
		Ok(())
	}
}

/// Implementation of Readable for a transaction Output, defines how to read
/// an Output from a binary stream.
impl Readable for Output {
	fn read(reader: &mut Reader) -> Result<Output, ser::Error> {
		let features =
			OutputFeatures::from_bits(reader.read_u8()?).ok_or(ser::Error::CorruptedData)?;

		Ok(Output {
			features: features,
			commit: Commitment::read(reader)?,
			proof: RangeProof::read(reader)?,
		})
	}
}

impl Output {
	/// Commitment for the output
	pub fn commitment(&self) -> Commitment {
		self.commit
	}

	/// Range proof for the output
	pub fn proof(&self) -> RangeProof {
		self.proof
	}

	/// Validates the range proof using the commitment
	pub fn verify_proof(&self) -> Result<(), secp::Error> {
		let secp = static_secp_instance();
		let secp = secp.lock().unwrap();
		match secp.verify_bullet_proof(self.commit, self.proof, None) {
			Ok(_) => Ok(()),
			Err(e) => Err(e),
		}
	}
}

/// An output_identifier can be build from either an input _or_ an output and
/// contains everything we need to uniquely identify an output being spent.
/// Needed because it is not sufficient to pass a commitment around.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OutputIdentifier {
	/// Output features (coinbase vs. regular transaction output)
	/// We need to include this when hashing to ensure coinbase maturity can be
	/// enforced.
	pub features: OutputFeatures,
	/// Output commitment
	pub commit: Commitment,
}

impl OutputIdentifier {
	/// Build a new output_identifier.
	pub fn new(features: OutputFeatures, commit: &Commitment) -> OutputIdentifier {
		OutputIdentifier {
			features: features,
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

	/// Converts this identifier to a full output, provided a RangeProof
	pub fn into_output(self, proof: RangeProof) -> Output {
		Output {
			features: self.features,
			commit: self.commit,
			proof: proof,
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
}

/// Ensure this is implemented to centralize hashing with indexes
impl PMMRable for OutputIdentifier {
	fn len() -> usize {
		1 + secp::constants::PEDERSEN_COMMITMENT_SIZE
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
		let features =
			OutputFeatures::from_bits(reader.read_u8()?).ok_or(ser::Error::CorruptedData)?;
		Ok(OutputIdentifier {
			commit: Commitment::read(reader)?,
			features: features,
		})
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use core::id::{ShortId, ShortIdentifiable};
	use keychain::{ExtKeychain, Keychain};
	use util::secp;

	#[test]
	fn test_kernel_ser_deser() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let commit = keychain.commit(5, &key_id).unwrap();

		// just some bytes for testing ser/deser
		let sig = secp::Signature::from_raw_data(&[0; 64]).unwrap();

		let kernel = TxKernel {
			features: KernelFeatures::DEFAULT_KERNEL,
			lock_height: 0,
			excess: commit,
			excess_sig: sig.clone(),
			fee: 10,
		};

		let mut vec = vec![];
		ser::serialize(&mut vec, &kernel).expect("serialized failed");
		let kernel2: TxKernel = ser::deserialize(&mut &vec[..]).unwrap();
		assert_eq!(kernel2.features, KernelFeatures::DEFAULT_KERNEL);
		assert_eq!(kernel2.lock_height, 0);
		assert_eq!(kernel2.excess, commit);
		assert_eq!(kernel2.excess_sig, sig.clone());
		assert_eq!(kernel2.fee, 10);

		// now check a kernel with lock_height serialize/deserialize correctly
		let kernel = TxKernel {
			features: KernelFeatures::DEFAULT_KERNEL,
			lock_height: 100,
			excess: commit,
			excess_sig: sig.clone(),
			fee: 10,
		};

		let mut vec = vec![];
		ser::serialize(&mut vec, &kernel).expect("serialized failed");
		let kernel2: TxKernel = ser::deserialize(&mut &vec[..]).unwrap();
		assert_eq!(kernel2.features, KernelFeatures::DEFAULT_KERNEL);
		assert_eq!(kernel2.lock_height, 100);
		assert_eq!(kernel2.excess, commit);
		assert_eq!(kernel2.excess_sig, sig.clone());
		assert_eq!(kernel2.fee, 10);
	}

	#[test]
	fn commit_consistency() {
		let keychain = ExtKeychain::from_seed(&[0; 32]).unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();

		let commit = keychain.commit(1003, &key_id).unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();

		let commit_2 = keychain.commit(1003, &key_id).unwrap();

		assert!(commit == commit_2);
	}

	#[test]
	fn input_short_id() {
		let keychain = ExtKeychain::from_seed(&[0; 32]).unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let commit = keychain.commit(5, &key_id).unwrap();

		let input = Input {
			features: OutputFeatures::DEFAULT_OUTPUT,
			commit: commit,
			block_hash: None,
			merkle_proof: None,
		};

		let block_hash = Hash::from_hex(
			"3a42e66e46dd7633b57d1f921780a1ac715e6b93c19ee52ab714178eb3a9f673",
		).unwrap();

		let nonce = 0;

		let short_id = input.short_id(&block_hash, nonce);
		assert_eq!(short_id, ShortId::from_hex("28fea5a693af").unwrap());

		// now generate the short_id for a *very* similar output (single feature flag
		// different) and check it generates a different short_id
		let input = Input {
			features: OutputFeatures::COINBASE_OUTPUT,
			commit: commit,
			block_hash: None,
			merkle_proof: None,
		};

		let short_id = input.short_id(&block_hash, nonce);
		assert_eq!(short_id, ShortId::from_hex("2df325971ab0").unwrap());
	}
}
