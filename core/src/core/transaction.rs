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

//! Transactions

use core::Committed;
use core::MerkleRow;
use core::hash::{Hash, Hashed};

use byteorder::{ByteOrder, BigEndian};
use secp::{self, Secp256k1, Message, Signature};
use secp::key::SecretKey;
use secp::pedersen::{RangeProof, Commitment};

use ser::{self, Reader, Writer, Readable, Writeable};

/// The maximum number of inputs or outputs a transaction may have
/// and be deserializable.
pub const MAX_IN_OUT_LEN: u64 = 50000;

/// A proof that a transaction sums to zero. Includes both the transaction's
/// Pedersen commitment and the signature, that guarantees that the commitments
/// amount to zero. The signature signs the fee, which is retained for
/// signature validation.
#[derive(Debug, Clone)]
pub struct TxProof {
	/// Remainder of the sum of all transaction commitments. If the transaction
	/// is well formed, amounts components should sum to zero and the remainder
	/// is hence a valid public key.
	pub remainder: Commitment,
	/// The signature proving the remainder is a valid public key, which signs
	/// the transaction fee.
	pub sig: Vec<u8>,
	/// Fee originally included in the transaction this proof is for.
	pub fee: u64,
}

impl Writeable for TxProof {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		try!(writer.write_fixed_bytes(&self.remainder));
		try!(writer.write_bytes(&self.sig));
		writer.write_u64(self.fee)
	}
}

impl Readable<TxProof> for TxProof {
	fn read(reader: &mut Reader) -> Result<TxProof, ser::Error> {
		let (remainder, sig, fee) = ser_multiread!(reader, read_33_bytes, read_vec, read_u64);
		Ok(TxProof {
			remainder: Commitment::from_vec(remainder),
			sig: sig,
			fee: fee,
		})
	}
}

impl TxProof {
	/// Verify the transaction proof validity. Entails handling the commitment
	/// as a public key and checking the signature verifies with the fee as
	/// message.
	pub fn verify(&self, secp: &Secp256k1) -> Result<(), secp::Error> {
		let msg = try!(Message::from_slice(&u64_to_32bytes(self.fee)));
		let pubk = try!(self.remainder.to_pubkey(secp));
		let sig = try!(Signature::from_der(secp, &self.sig));
		secp.verify(&msg, &sig, &pubk)
	}
}

/// A transaction
#[derive(Debug)]
pub struct Transaction {
	hash_mem: Option<Hash>,
	pub fee: u64,
	pub zerosig: Vec<u8>,
	pub inputs: Vec<Input>,
	pub outputs: Vec<Output>,
}

/// Implementation of Writeable for a fully blinded transaction, defines how to
/// write the transaction as binary.
impl Writeable for Transaction {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		ser_multiwrite!(writer,
		                [write_u64, self.fee],
		                [write_bytes, &self.zerosig],
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
impl Readable<Transaction> for Transaction {
	fn read(reader: &mut Reader) -> Result<Transaction, ser::Error> {
		let (fee, zerosig, input_len, output_len) =
			ser_multiread!(reader, read_u64, read_vec, read_u64, read_u64);

		// in case a facetious miner sends us more than what we can allocate
		if input_len > MAX_IN_OUT_LEN || output_len > MAX_IN_OUT_LEN {
			return Err(ser::Error::TooLargeReadErr("Too many inputs or outputs.".to_string()));
		}

		let inputs = try!((0..input_len).map(|_| Input::read(reader)).collect());
		let outputs = try!((0..output_len).map(|_| Output::read(reader)).collect());

		Ok(Transaction {
			fee: fee,
			zerosig: zerosig,
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
		let msg = try!(Message::from_slice(&u64_to_32bytes(self.fee)));
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
		let msg = try!(Message::from_slice(&u64_to_32bytes(self.fee)));
		let sig = try!(Signature::from_der(secp, &self.zerosig));
		try!(secp.verify(&msg, &sig, &pubk));

		Ok(TxProof {
			remainder: rsum,
			sig: self.zerosig.clone(),
			fee: self.fee,
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

/// Implementation of Writeable for a transaction Input, defines how to write
/// an Input as binary.
impl Writeable for Input {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		writer.write_fixed_bytes(&self.output_hash())
	}
}

/// Implementation of Readable for a transaction Input, defines how to read
/// an Input from a binary stream.
impl Readable<Input> for Input {
	fn read(reader: &mut Reader) -> Result<Input, ser::Error> {
		reader.read_fixed_bytes(32)
			.map(|h| Input::BareInput { output: Hash::from_vec(h) })
	}
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

#[derive(Debug, Copy, Clone)]
pub enum Output {
	BlindOutput {
		commit: Commitment,
		proof: RangeProof,
	},
	OvertOutput { value: u64, blindkey: SecretKey },
}

/// Implementation of Writeable for a transaction Output, defines how to write
/// an Output as binary.
impl Writeable for Output {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		// The hash of an output is only the hash of its commitment.
		try!(writer.write_fixed_bytes(&self.commitment().unwrap()));
		if writer.serialization_mode() == ser::SerializationMode::Full {
			try!(writer.write_bytes(&self.proof().unwrap().bytes()))
		}
		Ok(())
	}
}

/// Implementation of Readable for a transaction Output, defines how to read
/// an Output from a binary stream.
impl Readable<Output> for Output {
	fn read(reader: &mut Reader) -> Result<Output, ser::Error> {
		let (commit, proof) = ser_multiread!(reader, read_33_bytes, read_vec);
		Ok(Output::BlindOutput {
			commit: Commitment::from_vec(commit),
			proof: RangeProof::from_vec(proof),
		})
	}
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

#[cfg(test)]
mod test {
	use super::*;
	use core::hash::Hashed;
	use core::hash::ZERO_HASH;
	use core::test::{tx1i1o, tx2i1o};
	use ser::{deserialize, serialize};

	use secp::{self, Secp256k1};
	use secp::key::SecretKey;
	use rand::os::OsRng;

	fn new_secp() -> Secp256k1 {
		secp::Secp256k1::with_caps(secp::ContextFlag::Commit)
	}

	#[test]
	fn simple_tx_ser() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();

		let tx = tx2i1o(secp, &mut rng);
		let btx = tx.blind(&secp).unwrap();
		let mut vec = Vec::new();
		serialize(&mut vec, &btx).expect("serialized failed");
		assert!(vec.len() > 5320);
		assert!(vec.len() < 5340);
	}

	#[test]
	fn simple_tx_ser_deser() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();

		let tx = tx2i1o(secp, &mut rng);
		let btx = tx.blind(&secp).unwrap();
		let mut vec = Vec::new();
		serialize(&mut vec, &btx).expect("serialization failed");
		// let mut dtx = Transaction::read(&mut BinReader { source: &mut &vec[..]
		// }).unwrap();
		let dtx: Transaction = deserialize(&mut &vec[..]).unwrap();
		assert_eq!(dtx.fee, 1);
		assert_eq!(dtx.inputs.len(), 2);
		assert_eq!(dtx.outputs.len(), 1);
		assert_eq!(btx.hash(), dtx.hash());
	}

	#[test]
	fn tx_double_ser_deser() {
		// checks serializing doesn't mess up the tx and produces consistent results
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();

		let tx = tx2i1o(secp, &mut rng);
		let btx = tx.blind(&secp).unwrap();

		let mut vec = Vec::new();
		assert!(serialize(&mut vec, &btx).is_ok());
		let dtx: Transaction = deserialize(&mut &vec[..]).unwrap();

		let mut vec2 = Vec::new();
		assert!(serialize(&mut vec2, &btx).is_ok());
		let dtx2: Transaction = deserialize(&mut &vec2[..]).unwrap();

		assert_eq!(btx.hash(), dtx.hash());
		assert_eq!(dtx.hash(), dtx2.hash());
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
		let btx1 = tx1.blind(&secp).unwrap();

		let tx2 = tx1i1o(secp, &mut rng);
		let btx2 = tx2.blind(&secp).unwrap();

		if btx1.hash() == btx2.hash() {
			panic!("diff txs have same hash")
		}
	}
}
