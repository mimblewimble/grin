// Copyright 2019 The Grin Developers
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

//! Common test functions

use grin_core::core::{Block, BlockHeader, KernelFeatures, Transaction};
use grin_core::core::hash::DefaultHashable;
use grin_core::libtx::{
	build::{self, input, output},
	proof::{ProofBuild, ProofBuilder},
	reward,
};
use grin_core::pow::Difficulty;
use grin_core::ser::{self, FixedLength, PMMRable, Readable, Reader, Writeable, Writer};
use keychain::{Identifier, Keychain};

// utility producing a transaction with 2 inputs and a single outputs
#[allow(dead_code)]
pub fn tx2i1o() -> Transaction {
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	build::transaction(
		KernelFeatures::Plain { fee: 2 },
		vec![input(10, key_id1), input(11, key_id2), output(19, key_id3)],
		&keychain,
		&builder,
	)
	.unwrap()
}

// utility producing a transaction with a single input and output
#[allow(dead_code)]
pub fn tx1i1o() -> Transaction {
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);

	build::transaction(
		KernelFeatures::Plain { fee: 2 },
		vec![input(5, key_id1), output(3, key_id2)],
		&keychain,
		&builder,
	)
	.unwrap()
}

// utility producing a transaction with a single input
// and two outputs (one change output)
// Note: this tx has an "offset" kernel
#[allow(dead_code)]
pub fn tx1i2o() -> Transaction {
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	build::transaction(
		KernelFeatures::Plain { fee: 2 },
		vec![input(6, key_id1), output(3, key_id2), output(1, key_id3)],
		&keychain,
		&builder,
	)
	.unwrap()
}

// utility to create a block without worrying about the key or previous
// header
#[allow(dead_code)]
pub fn new_block<K, B>(
	txs: Vec<&Transaction>,
	keychain: &K,
	builder: &B,
	previous_header: &BlockHeader,
	key_id: &Identifier,
) -> Block
where
	K: Keychain,
	B: ProofBuild,
{
	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let reward_output = reward::output(keychain, builder, &key_id, fees, false).unwrap();
	Block::new(
		&previous_header,
		txs.into_iter().cloned().collect(),
		Difficulty::min(),
		reward_output,
	)
	.unwrap()
}

// utility producing a transaction that spends an output with the provided
// value and blinding key
#[allow(dead_code)]
pub fn txspend1i1o<K, B>(
	v: u64,
	keychain: &K,
	builder: &B,
	key_id1: Identifier,
	key_id2: Identifier,
) -> Transaction
where
	K: Keychain,
	B: ProofBuild,
{
	build::transaction(
		KernelFeatures::Plain { fee: 2 },
		vec![input(v, key_id1), output(3, key_id2)],
		keychain,
		builder,
	)
	.unwrap()
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TestElem(pub [u32; 4]);

impl DefaultHashable for TestElem {}

impl FixedLength for TestElem {
	const LEN: usize = 16;
}

impl PMMRable for TestElem {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		self.clone()
	}
}

impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		r#try!(writer.write_u32(self.0[0]));
		r#try!(writer.write_u32(self.0[1]));
		r#try!(writer.write_u32(self.0[2]));
		writer.write_u32(self.0[3])
	}
}

impl Readable for TestElem {
	fn read(reader: &mut dyn Reader) -> Result<TestElem, ser::Error> {
		Ok(TestElem([
			reader.read_u32()?,
			reader.read_u32()?,
			reader.read_u32()?,
			reader.read_u32()?,
		]))
	}
}
