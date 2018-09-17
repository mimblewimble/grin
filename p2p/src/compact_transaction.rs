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

use rand::{thread_rng, Rng};

use core::consensus::VerifySortOrder;
use core::core::hash::{Hash, Hashed};
use core::core::id::{ShortId, ShortIdentifiable};
use core::core::transaction::{Error, Transaction};
use core::ser::{self, read_multi, Readable, Reader, Writeable, Writer};

#[derive(Debug, Clone)]
pub struct CompactTransactionBody {
	pub new_kern_ids: Vec<ShortId>,
	pub req_kern_ids: Vec<ShortId>,
}

impl CompactTransactionBody {
	fn init(
		new_kern_ids: Vec<ShortId>,
		req_kern_ids: Vec<ShortId>,
		verify_sorted: bool,
	) -> Result<Self, Error> {
		let body = CompactTransactionBody {
			new_kern_ids,
			req_kern_ids,
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
		self.new_kern_ids.sort();
		self.req_kern_ids.sort();
	}

	/// "Lightweight" validation.
	fn validate_read(&self) -> Result<(), Error> {
		self.verify_sorted()?;
		Ok(())
	}

	// Verify everything is sorted in lexicographical order.
	fn verify_sorted(&self) -> Result<(), Error> {
		self.new_kern_ids.verify_sort_order()?;
		self.req_kern_ids.verify_sort_order()?;
		Ok(())
	}
}

impl Readable for CompactTransactionBody {
	fn read(reader: &mut Reader) -> Result<CompactTransactionBody, ser::Error> {
		let (new_kern_id_len, req_kern_id_len) = ser_multiread!(reader, read_u64, read_u64);

		let new_kern_ids = read_multi(reader, new_kern_id_len)?;
		let req_kern_ids = read_multi(reader, req_kern_id_len)?;

		// Initialize transaction block body, verifying sort order.
		let body = CompactTransactionBody::init(new_kern_ids, req_kern_ids, true)
			.map_err(|_| ser::Error::CorruptedData)?;

		Ok(body)
	}
}

impl Writeable for CompactTransactionBody {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u64, self.new_kern_ids.len() as u64],
			[write_u64, self.req_kern_ids.len() as u64]
		);
		self.new_kern_ids.write(writer)?;
		self.req_kern_ids.write(writer)?;
		Ok(())
	}
}

impl Into<CompactTransactionBody> for CompactTransaction {
	fn into(self) -> CompactTransactionBody {
		self.body
	}
}

#[derive(Debug, Clone)]
pub struct CompactTransaction {
	/// Hash of the latest block header (used as part of short_id generation).
	pub tx_hash: Hash,
	/// Nonce for connection specific short_ids.
	pub nonce: u64,
	/// Container for kern_ids in the compact transaction.
	body: CompactTransactionBody,

	pub tx: Option<Transaction>,
}

impl CompactTransaction {
	/// "Lightweight" validation.
	fn validate_read(&self) -> Result<(), Error> {
		self.body.validate_read()?;

		if let Some(ref tx) = self.tx {
			tx.validate_read()?;
		}

		Ok(())
	}

	/// Get kern_ids.
	pub fn new_kern_ids(&self) -> &Vec<ShortId> {
		&self.body.new_kern_ids
	}

	pub fn req_kern_ids(&self) -> &Vec<ShortId> {
		&self.body.req_kern_ids
	}

	pub fn add_kern_ids_to_request(&mut self, kern_ids: &Vec<ShortId>) {
		for x in kern_ids {
			self.body.req_kern_ids.push(x.clone())
		}
	}

	// TODO - is this wise?
	pub fn hash(&self) -> Hash {
		self.tx_hash
	}

	pub fn with_full_tx(self, tx: Transaction) -> CompactTransaction {
		// Swap the "required" kern_ids out for a full transaction.
		CompactTransaction {
			body: CompactTransactionBody {
				req_kern_ids: vec![],
				..self.body
			},
			tx: Some(tx),
			..self
		}
	}

	// When we send a compact_tx back to the original sender, requesting some full
	// tx(s) to help us hydrate the compact_tx.
	pub fn with_req_kern_ids(self, req_kern_ids: Vec<ShortId>) -> CompactTransaction {
		CompactTransaction {
			body: CompactTransactionBody {
				req_kern_ids,
				..self.body
			},
			..self
		}
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
		let body =
			CompactTransactionBody::init(kern_ids, vec![], false).expect("sorting, not verifying");

		CompactTransaction {
			tx_hash,
			nonce,
			body,
			tx: None,
		}
	}
}

impl Writeable for CompactTransaction {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.tx_hash.write(writer)?;
		writer.write_u64(self.nonce)?;
		self.body.write(writer)?;

		if let Some(ref tx) = self.tx {
			tx.write(writer)?;
		}

		Ok(())
	}
}

impl Readable for CompactTransaction {
	fn read(reader: &mut Reader) -> Result<CompactTransaction, ser::Error> {
		let tx_hash = Hash::read(reader)?;
		let nonce = reader.read_u64()?;
		let body = CompactTransactionBody::read(reader)?;

		// Read the associated full tx (if present).
		let tx = Transaction::read(reader).ok();

		let compact_tx = CompactTransaction {
			tx_hash,
			nonce,
			body,
			tx,
		};

		// Now validate the compact transaction and treat any validation error as corrupted data.
		compact_tx
			.validate_read()
			.map_err(|_| ser::Error::CorruptedData)?;

		Ok(compact_tx)
	}
}
