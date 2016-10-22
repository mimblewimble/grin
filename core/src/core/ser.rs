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

//! Binary stream serialization and deserialzation for core types from trusted
//! Write or Read implementations. Issues like starvation or too big sends are
//! expected to be handled upstream.

use time;

use std::io::{Write, Read};
use core::{self, hash};
use ser::*;

use secp::Signature;
use secp::key::SecretKey;
use secp::pedersen::{Commitment, RangeProof};

const MAX_IN_OUT_LEN: u64 = 50000;

macro_rules! impl_slice_bytes {
  ($byteable: ty) => {
    impl AsFixedBytes for $byteable {
      fn as_fixed_bytes(&self) -> &[u8] {
        &self[..]
      }
    }
  }
}

impl_slice_bytes!(SecretKey);
impl_slice_bytes!(Signature);
impl_slice_bytes!(Commitment);
impl_slice_bytes!(Vec<u8>);

impl AsFixedBytes for hash::Hash {
	fn as_fixed_bytes(&self) -> &[u8] {
		self.to_slice()
	}
}

impl AsFixedBytes for RangeProof {
	fn as_fixed_bytes(&self) -> &[u8] {
		&self.bytes()
	}
}

/// Implementation of Writeable for a transaction Input, defines how to write
/// an Input as binary.
impl Writeable for core::Input {
	fn write(&self, writer: &mut Writer) -> Option<Error> {
		writer.write_fixed_bytes(&self.output_hash())
	}
}

/// Implementation of Writeable for a transaction Output, defines how to write
/// an Output as binary.
impl Writeable for core::Output {
	fn write(&self, writer: &mut Writer) -> Option<Error> {
		try_m!(writer.write_fixed_bytes(&self.commitment().unwrap()));
		writer.write_vec(&mut self.proof().unwrap().bytes().to_vec())
	}
}

/// Implementation of Writeable for a fully blinded transaction, defines how to
/// write the transaction as binary.
impl Writeable for core::Transaction {
	fn write(&self, writer: &mut Writer) -> Option<Error> {
		try_m!(writer.write_u64(self.fee));
		try_m!(writer.write_vec(&mut self.zerosig.clone()));
		try_m!(writer.write_u64(self.inputs.len() as u64));
		try_m!(writer.write_u64(self.outputs.len() as u64));
		for inp in &self.inputs {
			try_m!(inp.write(writer));
		}
		for out in &self.outputs {
			try_m!(out.write(writer));
		}
		None
	}
}

impl Writeable for core::TxProof {
	fn write(&self, writer: &mut Writer) -> Option<Error> {
		try_m!(writer.write_fixed_bytes(&self.remainder));
		writer.write_vec(&mut self.sig.clone())
	}
}

/// Implementation of Writeable for a block, defines how to write the full
/// block as binary.
impl Writeable for core::Block {
	fn write(&self, writer: &mut Writer) -> Option<Error> {
		try_m!(self.header.write(writer));

		try_m!(writer.write_u64(self.inputs.len() as u64));
		try_m!(writer.write_u64(self.outputs.len() as u64));
		try_m!(writer.write_u64(self.proofs.len() as u64));
		for inp in &self.inputs {
			try_m!(inp.write(writer));
		}
		for out in &self.outputs {
			try_m!(out.write(writer));
		}
		for proof in &self.proofs {
			try_m!(proof.write(writer));
		}
		None
	}
}

/// Implementation of Readable for a transaction Input, defines how to read
/// an Input from a binary stream.
impl Readable<core::Input> for core::Input {
	fn read(reader: &mut Reader) -> Result<core::Input, Error> {
		reader.read_fixed_bytes(32)
			.map(|h| core::Input::BareInput { output: hash::Hash::from_vec(h) })
	}
}

/// Implementation of Readable for a transaction Output, defines how to read
/// an Output from a binary stream.
impl Readable<core::Output> for core::Output {
	fn read(reader: &mut Reader) -> Result<core::Output, Error> {
		let commit = try!(reader.read_fixed_bytes(33));
		let proof = try!(reader.read_vec());
		Ok(core::Output::BlindOutput {
			commit: Commitment::from_vec(commit),
			proof: RangeProof::from_vec(proof),
		})
	}
}

/// Implementation of Readable for a transaction, defines how to read a full
/// transaction from a binary stream.
impl Readable<core::Transaction> for core::Transaction {
	fn read(reader: &mut Reader) -> Result<core::Transaction, Error> {
		let fee = try!(reader.read_u64());
		let zerosig = try!(reader.read_vec());
		let input_len = try!(reader.read_u64());
		let output_len = try!(reader.read_u64());

		// in case a facetious miner sends us more than what we can allocate
		if input_len > MAX_IN_OUT_LEN || output_len > MAX_IN_OUT_LEN {
			return Err(Error::TooLargeReadErr("Too many inputs or outputs.".to_string()));
		}

		let inputs = try!((0..input_len).map(|_| core::Input::read(reader)).collect());
		let outputs = try!((0..output_len).map(|_| core::Output::read(reader)).collect());

		Ok(core::Transaction {
			fee: fee,
			zerosig: zerosig,
			inputs: inputs,
			outputs: outputs,
			..Default::default()
		})
	}
}

impl Readable<core::TxProof> for core::TxProof {
	fn read(reader: &mut Reader) -> Result<core::TxProof, Error> {
		let remainder = try!(reader.read_fixed_bytes(33));
		let sig = try!(reader.read_vec());
		Ok(core::TxProof {
			remainder: Commitment::from_vec(remainder),
			sig: sig,
		})
	}
}

/// Implementation of Readable for a block, defines how to read a full block
/// from a binary stream.
impl Readable<core::Block> for core::Block {
	fn read(reader: &mut Reader) -> Result<core::Block, Error> {
		let height = try!(reader.read_u64());
		let previous = try!(reader.read_fixed_bytes(32));
		let timestamp = try!(reader.read_i64());
		let utxo_merkle = try!(reader.read_fixed_bytes(32));
		let tx_merkle = try!(reader.read_fixed_bytes(32));
		let total_fees = try!(reader.read_u64());
		let nonce = try!(reader.read_u64());
		// cuckoo cycle of 42 nodes
		let mut pow = [0; core::PROOFSIZE];
		for n in 0..core::PROOFSIZE {
			pow[n] = try!(reader.read_u32());
		}
		let td = try!(reader.read_u64());

		let input_len = try!(reader.read_u64());
		let output_len = try!(reader.read_u64());
		let proof_len = try!(reader.read_u64());
		if input_len > MAX_IN_OUT_LEN || output_len > MAX_IN_OUT_LEN || proof_len > MAX_IN_OUT_LEN {
			return Err(Error::TooLargeReadErr("Too many inputs, outputs or proofs.".to_string()));
		}

		let inputs = try!((0..input_len).map(|_| core::Input::read(reader)).collect());
		let outputs = try!((0..output_len).map(|_| core::Output::read(reader)).collect());
		let proofs = try!((0..proof_len).map(|_| core::TxProof::read(reader)).collect());
		Ok(core::Block {
			header: core::BlockHeader {
				height: height,
				previous: hash::Hash::from_vec(previous),
				timestamp: time::at_utc(time::Timespec {
					sec: timestamp,
					nsec: 0,
				}),
				td: td,
				utxo_merkle: hash::Hash::from_vec(utxo_merkle),
				tx_merkle: hash::Hash::from_vec(tx_merkle),
				total_fees: total_fees,
				pow: core::Proof(pow),
				nonce: nonce,
			},
			inputs: inputs,
			outputs: outputs,
			proofs: proofs,
			..Default::default()
		})
	}
}

#[cfg(test)]
mod test {
	use ser::{serialize, deserialize};
	use secp;
	use secp::*;
	use secp::key::*;
	use core::*;
	use core::hash::ZERO_HASH;
	use rand::Rng;
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
		if let Some(e) = serialize(&mut vec, &btx) {
			panic!(e);
		}
		assert!(vec.len() > 5320);
		assert!(vec.len() < 5340);
	}

	#[test]
	fn simple_tx_ser_deser() {
		let mut rng = OsRng::new().unwrap();
		let ref secp = new_secp();

		let tx = tx2i1o(secp, &mut rng);
		let mut btx = tx.blind(&secp).unwrap();
		let mut vec = Vec::new();
		if let Some(e) = serialize(&mut vec, &btx) {
			panic!(e);
		}
		// let mut dtx = Transaction::read(&mut BinReader { source: &mut &vec[..]
		// }).unwrap();
		let mut dtx: Transaction = deserialize(&mut &vec[..]).unwrap();
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
		let mut btx = tx.blind(&secp).unwrap();

		let mut vec = Vec::new();
		assert!(serialize(&mut vec, &btx).is_none());
		let mut dtx: Transaction = deserialize(&mut &vec[..]).unwrap();

		let mut vec2 = Vec::new();
		assert!(serialize(&mut vec2, &btx).is_none());
		let mut dtx2: Transaction = deserialize(&mut &vec2[..]).unwrap();

		assert_eq!(btx.hash(), dtx.hash());
		assert_eq!(dtx.hash(), dtx2.hash());
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
}
