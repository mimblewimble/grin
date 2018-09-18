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

//! Compact Transactions.

use rand::{thread_rng, RngCore};

use consensus::VerifySortOrder;
use core::hash::{Hash, Hashed};
use core::id::{ShortId, ShortIdentifiable};
use core::transaction::{Error, Transaction};
use ser::{self, read_multi, Readable, Reader, Writeable, Writer};

/// A compact transaction body, wrapping a vec of kernel short_ids.
#[derive(Debug, Clone)]
pub struct CompactTransactionBody {
	/// The vec of kernel short_ids that constitute the full transaction.
	pub kern_ids: Vec<ShortId>,
}

impl CompactTransactionBody {
	fn init(kern_ids: Vec<ShortId>, verify_sorted: bool) -> Result<Self, Error> {
		let body = CompactTransactionBody { kern_ids };

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
		self.kern_ids.sort();
	}

	/// "Lightweight" validation.
	fn validate_read(&self) -> Result<(), Error> {
		self.verify_sorted()?;
		Ok(())
	}

	// Verify everything is sorted in lexicographical order.
	fn verify_sorted(&self) -> Result<(), Error> {
		self.kern_ids.verify_sort_order()?;
		Ok(())
	}
}

impl Readable for CompactTransactionBody {
	fn read(reader: &mut Reader) -> Result<CompactTransactionBody, ser::Error> {
		let kern_id_len = reader.read_u64()?;
		let kern_ids = read_multi(reader, kern_id_len)?;

		// Initialize transaction transaction body, verifying sort order.
		let body =
			CompactTransactionBody::init(kern_ids, true).map_err(|_| ser::Error::CorruptedData)?;

		Ok(body)
	}
}

impl Writeable for CompactTransactionBody {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.kern_ids.len() as u64)?;
		self.kern_ids.write(writer)?;
		Ok(())
	}
}

impl Into<CompactTransactionBody> for CompactTransaction {
	fn into(self) -> CompactTransactionBody {
		self.body
	}
}

/// A CompactTransaction is a vec of kernel short_ids with the
/// associated tx hash and nonce to allow kernels to be rehashed
/// and compared against these short_ids.
#[derive(Debug, Clone)]
pub struct CompactTransaction {
	/// Hash of the latest block header (used as part of short_id generation).
	pub tx_hash: Hash,
	/// Nonce for connection specific short_ids.
	pub nonce: u64,
	/// Container for kern_ids in the compact transaction.
	body: CompactTransactionBody,
}

impl CompactTransaction {
	/// "Lightweight" validation.
	fn validate_read(&self) -> Result<(), Error> {
		self.body.validate_read()?;
		Ok(())
	}

	/// Get kern_ids.
	pub fn kern_ids(&self) -> &Vec<ShortId> {
		&self.body.kern_ids
	}

	/// The hash of the compact transaction is the hash of the underlying transaction.
	/// TODO - is this wise?
	pub fn hash(&self) -> Hash {
		self.tx_hash
	}
}

impl From<Transaction> for CompactTransaction {
	fn from(tx: Transaction) -> Self {
		// TODO - Are we ok using the tx as the source of the hash for generating the short_ids?
		let tx_hash = tx.hash();

		// Generate a random nonce (short_ids specific to a particular peer connection).
		let nonce = thread_rng().next_u64();

		let mut kern_ids = vec![];

		for k in tx.kernels() {
			kern_ids.push(k.short_id(&tx_hash, nonce));
		}

		// Initialize a compact transaction body and sort everything.
		let body = CompactTransactionBody::init(kern_ids, false).expect("sorting, not verifying");

		CompactTransaction {
			tx_hash,
			nonce,
			body,
		}
	}
}

impl Writeable for CompactTransaction {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.tx_hash.write(writer)?;
		writer.write_u64(self.nonce)?;
		self.body.write(writer)?;
		Ok(())
	}
}

impl Readable for CompactTransaction {
	fn read(reader: &mut Reader) -> Result<CompactTransaction, ser::Error> {
		let tx_hash = Hash::read(reader)?;
		let nonce = reader.read_u64()?;
		let body = CompactTransactionBody::read(reader)?;

		let compact_tx = CompactTransaction {
			tx_hash,
			nonce,
			body,
		};

		// Now validate the compact transaction and treat any validation error as corrupted data.
		compact_tx
			.validate_read()
			.map_err(|_| ser::Error::CorruptedData)?;

		Ok(compact_tx)
	}
}
