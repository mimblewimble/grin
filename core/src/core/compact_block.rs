// Copyright 2021 The Grin Developers
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

//! Compact Blocks.

use crate::core::block::{Block, BlockHeader, Error, UntrustedBlockHeader};
use crate::core::hash::{DefaultHashable, Hashed};
use crate::core::id::ShortIdentifiable;
use crate::core::{Output, ShortId, TxKernel};
use crate::ser::{self, read_multi, Readable, Reader, VerifySortedAndUnique, Writeable, Writer};
use rand::{thread_rng, Rng};

/// Container for full (full) outputs and kernels and kern_ids for a compact block.
#[derive(Debug, Clone)]
pub struct CompactBlockBody {
	/// List of full outputs - specifically the coinbase output(s)
	pub out_full: Vec<Output>,
	/// List of full kernels - specifically the coinbase kernel(s)
	pub kern_full: Vec<TxKernel>,
	/// List of transaction kernels, excluding those in the full list
	/// (short_ids)
	pub kern_ids: Vec<ShortId>,
}

impl CompactBlockBody {
	fn init(
		out_full: Vec<Output>,
		kern_full: Vec<TxKernel>,
		kern_ids: Vec<ShortId>,
		verify_sorted: bool,
	) -> Result<Self, Error> {
		let body = CompactBlockBody {
			out_full,
			kern_full,
			kern_ids,
		};

		if verify_sorted {
			// If we are verifying sort order then verify and
			// return an error if not sorted lexicographically.
			body.verify_sorted()?;
			Ok(body)
		} else {
			// If we are not verifying sort order then sort in place and return.
			let mut body = body;
			body.sort();
			Ok(body)
		}
	}

	/// Sort everything.
	fn sort(&mut self) {
		self.out_full.sort_unstable();
		self.kern_full.sort_unstable();
		self.kern_ids.sort_unstable();
	}

	/// "Lightweight" validation.
	fn validate_read(&self) -> Result<(), Error> {
		self.verify_sorted()?;
		Ok(())
	}

	// Verify everything is sorted in lexicographical order and no duplicates present.
	fn verify_sorted(&self) -> Result<(), Error> {
		self.out_full.verify_sorted_and_unique()?;
		self.kern_full.verify_sorted_and_unique()?;
		self.kern_ids.verify_sorted_and_unique()?;
		Ok(())
	}
}

impl Readable for CompactBlockBody {
	fn read<R: Reader>(reader: &mut R) -> Result<CompactBlockBody, ser::Error> {
		let (out_full_len, kern_full_len, kern_id_len) =
			ser_multiread!(reader, read_u64, read_u64, read_u64);

		let out_full = read_multi(reader, out_full_len)?;
		let kern_full = read_multi(reader, kern_full_len)?;
		let kern_ids = read_multi(reader, kern_id_len)?;

		// Initialize compact block body, verifying sort order.
		let body = CompactBlockBody::init(out_full, kern_full, kern_ids, true)
			.map_err(|_| ser::Error::CorruptedData)?;

		Ok(body)
	}
}

impl Writeable for CompactBlockBody {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u64, self.out_full.len() as u64],
			[write_u64, self.kern_full.len() as u64],
			[write_u64, self.kern_ids.len() as u64]
		);

		self.out_full.write(writer)?;
		self.kern_full.write(writer)?;
		self.kern_ids.write(writer)?;

		Ok(())
	}
}

impl Into<CompactBlockBody> for CompactBlock {
	fn into(self) -> CompactBlockBody {
		self.body
	}
}

/// Compact representation of a full block.
/// Each input/output/kernel is represented as a short_id.
/// A node is reasonably likely to have already seen all tx data (tx broadcast
/// before block) and can go request missing tx data from peers if necessary to
/// hydrate a compact block into a full block.
#[derive(Debug, Clone)]
pub struct CompactBlock {
	/// The header with metadata and commitments to the rest of the data
	pub header: BlockHeader,
	/// Nonce for connection specific short_ids
	pub nonce: u64,
	/// Container for out_full, kern_full and kern_ids in the compact block.
	body: CompactBlockBody,
}

impl DefaultHashable for CompactBlock {}

impl CompactBlock {
	/// "Lightweight" validation.
	fn validate_read(&self) -> Result<(), Error> {
		self.body.validate_read()?;
		Ok(())
	}

	/// Get kern_ids
	pub fn kern_ids(&self) -> &[ShortId] {
		&self.body.kern_ids
	}

	/// Get full (coinbase) kernels
	pub fn kern_full(&self) -> &[TxKernel] {
		&self.body.kern_full
	}

	/// Get full (coinbase) outputs
	pub fn out_full(&self) -> &[Output] {
		&self.body.out_full
	}
}

impl From<Block> for CompactBlock {
	fn from(block: Block) -> Self {
		let header = block.header.clone();
		let nonce = thread_rng().gen();

		let out_full = block
			.outputs()
			.iter()
			.filter(|x| x.is_coinbase())
			.cloned()
			.collect::<Vec<_>>();

		let mut kern_full = vec![];
		let mut kern_ids = vec![];

		for k in block.kernels() {
			if k.is_coinbase() {
				kern_full.push(k.clone());
			} else {
				kern_ids.push(k.short_id(&header.hash(), nonce));
			}
		}

		// Initialize a compact block body and sort everything.
		let body = CompactBlockBody::init(out_full, kern_full, kern_ids, false)
			.expect("sorting, not verifying");

		CompactBlock {
			header,
			nonce,
			body,
		}
	}
}

/// Implementation of Writeable for a compact block, defines how to write the
/// block to a binary writer. Differentiates between writing the block for the
/// purpose of full serialization and the one of just extracting a hash.
impl Writeable for CompactBlock {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.header.write(writer)?;

		if writer.serialization_mode() != ser::SerializationMode::Hash {
			writer.write_u64(self.nonce)?;
			self.body.write(writer)?;
		}

		Ok(())
	}
}

/// Implementation of Readable for a compact block, defines how to read a
/// compact block from a binary stream.
impl Readable for CompactBlock {
	fn read<R: Reader>(reader: &mut R) -> Result<CompactBlock, ser::Error> {
		let header = BlockHeader::read(reader)?;
		let nonce = reader.read_u64()?;
		let body = CompactBlockBody::read(reader)?;

		Ok(CompactBlock {
			header,
			nonce,
			body,
		})
	}
}

impl From<UntrustedCompactBlock> for CompactBlock {
	fn from(ucb: UntrustedCompactBlock) -> Self {
		ucb.0
	}
}

/// Compackt block which does lightweight validation as part of deserialization,
/// it supposed to be used when we can't trust the channel (eg network)
pub struct UntrustedCompactBlock(CompactBlock);

/// Implementation of Readable for an untrusted compact block, defines how to read a
/// compact block from a binary stream.
impl Readable for UntrustedCompactBlock {
	fn read<R: Reader>(reader: &mut R) -> Result<UntrustedCompactBlock, ser::Error> {
		let header = UntrustedBlockHeader::read(reader)?;
		let nonce = reader.read_u64()?;
		let body = CompactBlockBody::read(reader)?;

		let cb = CompactBlock {
			header: header.into(),
			nonce,
			body,
		};

		// Now validate the compact block and treat any validation error as corrupted data.
		cb.validate_read().map_err(|_| ser::Error::CorruptedData)?;

		Ok(UntrustedCompactBlock(cb))
	}
}
