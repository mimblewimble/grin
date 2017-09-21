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

use byteorder::{ByteOrder, BigEndian};
use secp::{self, Secp256k1, Message, Signature};
use secp::pedersen::{RangeProof, Commitment};

use core::Committed;
use core::MerkleRow;
use core::hash::{Hash, Hashed};
use ser::{self, Reader, Writer, Readable, Writeable};

bitflags! {
    /// Options for a kernel's structure or use
    pub flags KernelFeatures: u8 {
        /// No flags
        const DEFAULT_KERNEL = 0b00000000,
        /// Kernel matching a coinbase output
        const COINBASE_KERNEL = 0b00000001,
    }
}

/// A proof that a transaction sums to zero. Includes both the transaction's
/// Pedersen commitment and the signature, that guarantees that the commitments
/// amount to zero. The signature signs the fee, which is retained for
/// signature validation.
#[derive(Debug, Clone, PartialEq)]
pub struct TxKernel {
	/// Options for a kernel's structure or use
	pub features: KernelFeatures,
	/// Remainder of the sum of all transaction commitments. If the transaction
	/// is well formed, amounts components should sum to zero and the excess
	/// is hence a valid public key.
	pub excess: Commitment,
	/// The signature proving the excess is a valid public key, which signs
	/// the transaction fee.
	pub excess_sig: Vec<u8>,
	/// Fee originally included in the transaction this proof is for.
	pub fee: u64,
}

impl Writeable for TxKernel {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(writer,
		                [write_u8, self.features.bits()],
		                [write_fixed_bytes, &self.excess],
		                [write_bytes, &self.excess_sig],
		                [write_u64, self.fee]);
		Ok(())
	}
}

impl Readable for TxKernel {
	fn read(reader: &mut Reader) -> Result<TxKernel, ser::Error> {
		Ok(TxKernel {
			features:
				KernelFeatures::from_bits(reader.read_u8()?).ok_or(ser::Error::CorruptedData)?,
			excess: Commitment::read(reader)?,
			excess_sig: reader.read_vec()?,
			fee: reader.read_u64()?,
		})
	}
}

impl TxKernel {
	/// Verify the transaction proof validity. Entails handling the commitment
	/// as a public key and checking the signature verifies with the fee as
	/// message.
	pub fn verify(&self, secp: &Secp256k1) -> Result<(), secp::Error> {
		let msg = try!(Message::from_slice(&u64_to_32bytes(self.fee)));
		let sig = try!(Signature::from_der(secp, &self.excess_sig));
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
	/// The signature proving the excess is a valid public key, which signs
	/// the transaction fee.
	pub excess_sig: Vec<u8>,
}

/// Implementation of Writeable for a fully blinded transaction, defines how to
/// write the transaction as binary.
impl Writeable for Transaction {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(writer,
		                [write_u64, self.fee],
		                [write_bytes, &self.excess_sig],
		                [write_u64, self.inputs.len() as u64],
		                [write_u64, self.outputs.len() as u64]);
		for inp in &self.inputs {
			try!(inp.write(writer));
		}
		for out in &self.outputs {
			try!(out.write(writer));
		}
		Ok(())
	}
}

/// Implementation of Readable for a transaction, defines how to read a full
/// transaction from a binary stream.
impl Readable for Transaction {
	fn read(reader: &mut Reader) -> Result<Transaction, ser::Error> {
		let (fee, excess_sig, input_len, output_len) =
			ser_multiread!(reader, read_u64, read_vec, read_u64, read_u64);

		let inputs = try!((0..input_len).map(|_| Input::read(reader)).collect());
		let outputs = try!((0..output_len).map(|_| Output::read(reader)).collect());

		Ok(Transaction {
			fee: fee,
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
			excess_sig: vec![],
			inputs: vec![],
			outputs: vec![],
		}
	}

	/// Creates a new transaction initialized with the provided inputs,
	/// outputs and fee.
	pub fn new(inputs: Vec<Input>, outputs: Vec<Output>, fee: u64) -> Transaction {
		Transaction {
			fee: fee,
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
		Transaction { inputs: new_ins, ..self }
	}

	/// Builds a new transaction with the provided output added. Existing
	/// outputs, if any, are kept intact.
	pub fn with_output(self, output: Output) -> Transaction {
		let mut new_outs = self.outputs;
		new_outs.push(output);
		Transaction { outputs: new_outs, ..self }
	}

	/// Builds a new transaction with the provided fee.
	pub fn with_fee(self, fee: u64) -> Transaction {
		Transaction { fee: fee, ..self }
	}

	/// The verification for a MimbleWimble transaction involves getting the
	/// excess of summing all commitments and using it as a public key
	/// to verify the embedded signature. The rational is that if the values
	/// sum to zero as they should in r.G + v.H then only k.G the excess
	/// of the sum of r.G should be left. And r.G is the definition of a
	/// public key generated using r as a private key.
	pub fn verify_sig(&self, secp: &Secp256k1) -> Result<TxKernel, secp::Error> {
		let rsum = self.sum_commitments(secp)?;

		let msg = Message::from_slice(&u64_to_32bytes(self.fee))?;
		let sig = Signature::from_der(secp, &self.excess_sig)?;

		// pretend the sum is a public key (which it is, being of the form r.G) and
		// verify the transaction sig with it
		//
		// we originally converted the commitment to a pubkey here (commitment to zero)
		// and then passed the pubkey to secp.verify()
		// the secp api no longer allows us to do this so we have wrapped the complexity
		// of generating a publick key from a commitment behind verify_from_commit
		secp.verify_from_commit(&msg, &sig, &rsum)?;

		Ok(TxKernel {
			features: DEFAULT_KERNEL,
			excess: rsum,
			excess_sig: self.excess_sig.clone(),
			fee: self.fee,
		})
	}

	/// Validates all relevant parts of a fully built transaction. Checks the
	/// excess value against the signature as well as range proofs for each
	/// output.
	pub fn validate(&self, secp: &Secp256k1) -> Result<TxKernel, secp::Error> {
		for out in &self.outputs {
			out.verify_proof(secp)?;
		}
		self.verify_sig(secp)
	}
}

/// A transaction input, mostly a reference to an output being spent by the
/// transaction.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Input(pub Commitment);

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
        /// Output is a coinbase output, has fixed amount and must not be spent until maturity
        const COINBASE_OUTPUT = 0b00000001,
    }
}

/// Output for a transaction, defining the new ownership of coins that are being
/// transferred. The commitment is a blinded value for the output while the
/// range proof guarantees the commitment includes a positive value without
/// overflow and the ownership of the private key.
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
	/// Options for an output's structure or use
	pub features: OutputFeatures,
	/// The homomorphic commitment representing the output's amount
	pub commit: Commitment,
	/// A proof that the commitment is in the right range
	pub proof: RangeProof,
}

/// Implementation of Writeable for a transaction Output, defines how to write
/// an Output as binary.
impl Writeable for Output {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(writer,
		                [write_u8, self.features.bits()],
		                [write_fixed_bytes, &self.commit]);
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
		Ok(Output {
			features:
				OutputFeatures::from_bits(reader.read_u8()?).ok_or(ser::Error::CorruptedData)?,
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
	pub fn verify_proof(&self, secp: &Secp256k1) -> Result<(), secp::Error> {
		/// secp.verify_range_proof returns range if and only if both min_value and max_value less than 2^64
		/// since group order is much larger (~2^256) we can be sure overflow is not the case
		secp.verify_range_proof(self.commit, self.proof).map(|_| ())
	}
}

/// Utility function to calculate the Merkle root of vectors of inputs and
/// outputs.
pub fn merkle_inputs_outputs(inputs: &Vec<Input>, outputs: &Vec<Output>) -> Hash {
	let mut all_hs = map_vec!(inputs, |inp| inp.hash());
	all_hs.append(&mut map_vec!(outputs, |out| out.hash()));
	MerkleRow::new(all_hs).root()
}

fn u64_to_32bytes(n: u64) -> [u8; 32] {
	let mut bytes = [0; 32];
	BigEndian::write_u64(&mut bytes[24..32], n);
	bytes
}
