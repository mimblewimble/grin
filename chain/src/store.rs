// Copyright 2020 The Grin Developers
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

//! Implements storage primitives required by the chain

use crate::core::consensus::HeaderInfo;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::{Block, BlockHeader, BlockSums};
use crate::core::pow::Difficulty;
use crate::core::ser::{self, ProtocolVersion, Readable, Reader, Writeable, Writer};
use crate::types::{CommitPos, OutputPos, Tip};
use crate::util::secp::pedersen::Commitment;
use croaring::Bitmap;
use enum_primitive::FromPrimitive;
use grin_store as store;
use grin_store::{option_to_not_found, to_key, to_key_u64, Error, SerIterator};
use std::convert::TryInto;
use std::sync::Arc;

const STORE_SUBPATH: &str = "chain";

const BLOCK_HEADER_PREFIX: u8 = b'h';
const BLOCK_PREFIX: u8 = b'b';
const HEAD_PREFIX: u8 = b'H';
const TAIL_PREFIX: u8 = b'T';
const HEADER_HEAD_PREFIX: u8 = b'G';
const OUTPUT_POS_PREFIX: u8 = b'p';
const NEW_PLAIN_OUTPUT_POS_PREFIX: u8 = b'P';
const NEW_COINBASE_OUTPUT_POS_PREFIX: u8 = b'C';

const KERNEL_POS_PREFIX: u8 = b'K';

const BLOCK_INPUT_BITMAP_PREFIX: u8 = b'B';
const BLOCK_SUMS_PREFIX: u8 = b'M';
const BLOCK_SPENT_PREFIX: u8 = b'S';

/// All chain-related database operations
pub struct ChainStore {
	db: store::Store,
}

impl ChainStore {
	/// Create new chain store
	pub fn new(db_root: &str) -> Result<ChainStore, Error> {
		let db = store::Store::new(db_root, None, Some(STORE_SUBPATH), None)?;
		Ok(ChainStore { db })
	}

	/// Create a new instance of the chain store based on this instance
	/// but with the provided protocol version. This is used when migrating
	/// data in the db to a different protocol version, reading using one version and
	/// writing back to the db with a different version.
	pub fn with_version(&self, version: ProtocolVersion) -> ChainStore {
		let db_with_version = self.db.with_version(version);
		ChainStore {
			db: db_with_version,
		}
	}

	/// The current chain head.
	pub fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[HEAD_PREFIX]), || "HEAD".to_owned())
	}

	/// The current header head (may differ from chain head).
	pub fn header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[HEADER_HEAD_PREFIX]), || {
			"HEADER_HEAD".to_owned()
		})
	}

	/// The current chain "tail" (earliest block in the store).
	pub fn tail(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[TAIL_PREFIX]), || "TAIL".to_owned())
	}

	/// Header of the block at the head of the block chain (not the same thing as header_head).
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.get_block_header(&self.head()?.last_block_h)
	}

	/// Get full block.
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_PREFIX, h)), || {
			format!("BLOCK: {}", h)
		})
	}

	/// Does this full block exist?
	pub fn block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, h))
	}

	/// Get block_sums for the block hash.
	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_SUMS_PREFIX, h)), || {
			format!("Block sums for block: {}", h)
		})
	}

	/// Get previous header.
	pub fn get_previous_header(&self, header: &BlockHeader) -> Result<BlockHeader, Error> {
		self.get_block_header(&header.prev_hash)
	}

	/// Get block header.
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_HEADER_PREFIX, h)), || {
			format!("BLOCK HEADER: {}", h)
		})
	}

	/// Get PMMR pos for the given output commitment.
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		match self.get_output_pos_height(commit)? {
			Some((pos, _)) => Ok(pos),
			None => Err(Error::NotFoundErr(format!(
				"Output position for: {:?}",
				commit
			))),
		}
	}

	/// Get PMMR pos and block height for the given output commitment.
	pub fn get_output_pos_height(&self, commit: &Commitment) -> Result<Option<(u64, u64)>, Error> {
		self.db.get_ser(&to_key(OUTPUT_POS_PREFIX, commit))
	}

	/// Builds a new batch to be used with this store.
	pub fn batch(&self) -> Result<Batch<'_>, Error> {
		Ok(Batch {
			db: self.db.batch()?,
		})
	}
}

/// An atomic batch in which all changes can be committed all at once or
/// discarded on error.
pub struct Batch<'a> {
	db: store::Batch<'a>,
}

impl<'a> Batch<'a> {
	/// The head.
	pub fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[HEAD_PREFIX]), || "HEAD".to_owned())
	}

	/// The tail.
	pub fn tail(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[TAIL_PREFIX]), || "TAIL".to_owned())
	}

	/// The current header head (may differ from chain head).
	pub fn header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[HEADER_HEAD_PREFIX]), || {
			"HEADER_HEAD".to_owned()
		})
	}

	/// Header of the block at the head of the block chain (not the same thing as header_head).
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.get_block_header(&self.head()?.last_block_h)
	}

	/// Save body head to db.
	pub fn save_body_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&[HEAD_PREFIX], t)
	}

	/// Save body "tail" to db.
	pub fn save_body_tail(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&[TAIL_PREFIX], t)
	}

	/// Save header head to db.
	pub fn save_header_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&[HEADER_HEAD_PREFIX], t)
	}

	/// get block
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_PREFIX, h)), || {
			format!("Block with hash: {}", h)
		})
	}

	/// Does the block exist?
	pub fn block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, h))
	}

	/// Save the block to the db.
	/// Note: the block header is not saved to the db here, assumes this has already been done.
	pub fn save_block(&self, b: &Block) -> Result<(), Error> {
		self.db.put_ser(&to_key(BLOCK_PREFIX, b.hash())[..], b)?;
		Ok(())
	}

	/// We maintain a "spent" index for each full block to allow the output_pos
	/// to be easily reverted during rewind.
	pub fn save_spent_index(&self, h: &Hash, spent: &Vec<OutputPos>) -> Result<(), Error> {
		self.db.put_ser(&to_key(BLOCK_SPENT_PREFIX, h)[..], spent)?;
		Ok(())
	}

	/// Migrate a block stored in the db by serializing it using the provided protocol version.
	/// Block may have been read using a previous protocol version but we do not actually care.
	pub fn migrate_block(&self, b: &Block, version: ProtocolVersion) -> Result<(), Error> {
		self.db
			.put_ser_with_version(&to_key(BLOCK_PREFIX, &mut b.hash())[..], b, version)?;
		Ok(())
	}

	/// Low level function to delete directly by raw key.
	pub fn delete(&self, key: &[u8]) -> Result<(), Error> {
		self.db.delete(key)
	}

	/// Delete a full block. Does not delete any record associated with a block
	/// header.
	pub fn delete_block(&self, bh: &Hash) -> Result<(), Error> {
		self.db.delete(&to_key(BLOCK_PREFIX, bh)[..])?;

		// Best effort at deleting associated data for this block.
		// Not an error if these fail.
		{
			let _ = self.delete_block_sums(bh);
			let _ = self.delete_spent_index(bh);
		}

		Ok(())
	}

	/// Save block header to db.
	pub fn save_block_header(&self, header: &BlockHeader) -> Result<(), Error> {
		let hash = header.hash();

		// Store the header itself indexed by hash.
		self.db
			.put_ser(&to_key(BLOCK_HEADER_PREFIX, hash)[..], header)?;

		Ok(())
	}

	/// Save output_pos and block height to index.
	pub fn save_output_pos_height(
		&self,
		commit: &Commitment,
		pos: u64,
		height: u64,
	) -> Result<(), Error> {
		self.db
			.put_ser(&to_key(OUTPUT_POS_PREFIX, commit)[..], &(pos, height))
	}

	/// Delete the output_pos index entry for a spent output.
	pub fn delete_output_pos_height(&self, commit: &Commitment) -> Result<(), Error> {
		self.db.delete(&to_key(OUTPUT_POS_PREFIX, commit))
	}

	/// When using the output_pos iterator we have access to the index keys but not the
	/// original commitment that the key is constructed from. So we need a way of comparing
	/// a key with another commitment without reconstructing the commitment from the key bytes.
	pub fn is_match_output_pos_key(&self, key: &[u8], commit: &Commitment) -> bool {
		let commit_key = to_key(OUTPUT_POS_PREFIX, commit);
		commit_key == key
	}

	/// Iterator over the output_pos index.
	pub fn output_pos_iter(&self) -> Result<SerIterator<(u64, u64)>, Error> {
		let key = to_key(OUTPUT_POS_PREFIX, "");
		self.db.iter(&key)
	}

	/// Get output_pos from index.
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		match self.get_output_pos_height(commit)? {
			Some((pos, _)) => Ok(pos),
			None => Err(Error::NotFoundErr(format!(
				"Output position for: {:?}",
				commit
			))),
		}
	}

	/// Get output_pos and block height from index.
	pub fn get_output_pos_height(&self, commit: &Commitment) -> Result<Option<(u64, u64)>, Error> {
		self.db.get_ser(&to_key(OUTPUT_POS_PREFIX, commit))
	}

	/// Get the previous header.
	pub fn get_previous_header(&self, header: &BlockHeader) -> Result<BlockHeader, Error> {
		self.get_block_header(&header.prev_hash)
	}

	/// Get block header.
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_HEADER_PREFIX, h)), || {
			format!("BLOCK HEADER: {}", h)
		})
	}

	/// Delete the block spent index.
	fn delete_spent_index(&self, bh: &Hash) -> Result<(), Error> {
		// Clean up the legacy input bitmap as well.
		let _ = self.db.delete(&to_key(BLOCK_INPUT_BITMAP_PREFIX, bh));

		self.db.delete(&to_key(BLOCK_SPENT_PREFIX, bh))
	}

	/// Save block_sums for the block.
	pub fn save_block_sums(&self, h: &Hash, sums: BlockSums) -> Result<(), Error> {
		self.db.put_ser(&to_key(BLOCK_SUMS_PREFIX, h)[..], &sums)
	}

	/// Get block_sums for the block.
	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_SUMS_PREFIX, h)), || {
			format!("Block sums for block: {}", h)
		})
	}

	/// Delete the block_sums for the block.
	fn delete_block_sums(&self, bh: &Hash) -> Result<(), Error> {
		self.db.delete(&to_key(BLOCK_SUMS_PREFIX, bh))
	}

	/// Get the block input bitmap based on our spent index.
	/// Fallback to legacy block input bitmap from the db.
	pub fn get_block_input_bitmap(&self, bh: &Hash) -> Result<Bitmap, Error> {
		if let Ok(spent) = self.get_spent_index(bh) {
			let bitmap = spent
				.into_iter()
				.map(|x| x.pos.try_into().unwrap())
				.collect();
			Ok(bitmap)
		} else {
			self.get_legacy_input_bitmap(bh)
		}
	}

	fn get_legacy_input_bitmap(&self, bh: &Hash) -> Result<Bitmap, Error> {
		option_to_not_found(
			self.db
				.get_with(&to_key(BLOCK_INPUT_BITMAP_PREFIX, bh), Bitmap::deserialize),
			|| "legacy block input bitmap".to_string(),
		)
	}

	/// Get the "spent index" from the db for the specified block.
	/// If we need to rewind a block then we use this to "unspend" the spent outputs.
	pub fn get_spent_index(&self, bh: &Hash) -> Result<Vec<OutputPos>, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_SPENT_PREFIX, bh)), || {
			format!("spent index: {}", bh)
		})
	}

	/// Commits this batch. If it's a child batch, it will be merged with the
	/// parent, otherwise the batch is written to db.
	pub fn commit(self) -> Result<(), Error> {
		self.db.commit()
	}

	/// Creates a child of this batch. It will be merged with its parent on
	/// commit, abandoned otherwise.
	pub fn child(&mut self) -> Result<Batch<'_>, Error> {
		Ok(Batch {
			db: self.db.child()?,
		})
	}

	/// An iterator to all block in db
	pub fn blocks_iter(&self) -> Result<SerIterator<Block>, Error> {
		let key = to_key(BLOCK_PREFIX, "");
		self.db.iter(&key)
	}
}

enum_from_primitive! {
	#[derive(Copy, Clone, Debug, PartialEq)]
	enum LinkedListVariant {
		Unique = 0,
		Multi = 1,
	}
}

impl Writeable for LinkedListVariant {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(*self as u8)
	}
}

impl Readable for LinkedListVariant {
	fn read(reader: &mut dyn Reader) -> Result<LinkedListVariant, ser::Error> {
		LinkedListVariant::from_u8(reader.read_u8()?).ok_or(ser::Error::CorruptedData)
	}
}

enum_from_primitive! {
	#[derive(Copy, Clone, Debug, PartialEq)]
	enum ListEntryVariant {
		Head = 2,
		Tail = 3,
		Middle = 4,
	}
}

impl Writeable for ListEntryVariant {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(*self as u8)
	}
}

impl Readable for ListEntryVariant {
	fn read(reader: &mut dyn Reader) -> Result<ListEntryVariant, ser::Error> {
		ListEntryVariant::from_u8(reader.read_u8()?).ok_or(ser::Error::CorruptedData)
	}
}

pub trait FooLinkedList {
	/// List type
	type List: Readable + Writeable;

	/// List entry type
	type Entry: FooListEntry;

	fn list_key(&self, commit: Commitment) -> Vec<u8>;

	fn entry_key(&self, commit: Commitment, pos: u64) -> Vec<u8>;

	/// Returns either a "unique" with embedded "pos" or a "list" with "head" and "tail".
	/// Key is "prefix|commit".
	/// Note the key for an individual entry in the list is "prefix|commit|pos".
	fn get_list(&self, batch: &Batch<'_>, commit: Commitment) -> Result<Option<Self::List>, Error> {
		batch.db.get_ser(&self.list_key(commit))
	}

	/// Returns one of "head", "tail" or "middle" entry variants.
	/// Key is "prefix|commit|pos".
	fn get_entry(
		&self,
		batch: &Batch<'_>,
		commit: Commitment,
		pos: u64,
	) -> Result<Option<Self::Entry>, Error> {
		batch.db.get_ser(&self.entry_key(commit, pos))
	}

	fn push_entry(
		&self,
		batch: &Batch<'_>,
		commit: Commitment,
		new_pos: <Self::Entry as FooListEntry>::Pos,
	) -> Result<(), Error>;

	fn pop_entry(
		&self,
		batch: &Batch<'_>,
		commit: Commitment,
	) -> Result<Option<<Self::Entry as FooListEntry>::Pos>, Error>;
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum LinkedList<T> {
	Unique { pos: T },
	Multi { head: u64, tail: u64 },
}

impl<T> Writeable for LinkedList<T>
where
	T: Writeable,
{
	/// Write first byte representing the variant, followed by variant specific data.
	/// "Unique" is optimized with embedded "pos".
	/// "Multi" has references to "head" and "tail".
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		match self {
			LinkedList::Unique { pos } => {
				LinkedListVariant::Unique.write(writer)?;
				pos.write(writer)?;
			}
			LinkedList::Multi { head, tail } => {
				LinkedListVariant::Multi.write(writer)?;
				writer.write_u64(*head)?;
				writer.write_u64(*tail)?;
			}
		}
		Ok(())
	}
}

impl<T> Readable for LinkedList<T>
where
	T: Readable,
{
	/// Read the first byte to determine what needs to be read beyond that.
	fn read(reader: &mut dyn Reader) -> Result<LinkedList<T>, ser::Error> {
		let entry = match LinkedListVariant::read(reader)? {
			LinkedListVariant::Unique => LinkedList::Unique {
				pos: T::read(reader)?,
			},
			LinkedListVariant::Multi => LinkedList::Multi {
				head: reader.read_u64()?,
				tail: reader.read_u64()?,
			},
		};
		Ok(entry)
	}
}

pub struct MyLinkedList<T> {
	phantom: std::marker::PhantomData<*const T>,
	prefix: u8,
}

pub fn output_plain_index() -> MyLinkedList<OutputPos> {
	MyLinkedList {
		phantom: std::marker::PhantomData,
		prefix: NEW_PLAIN_OUTPUT_POS_PREFIX,
	}
}

pub fn output_coinbase_index() -> MyLinkedList<OutputPos> {
	MyLinkedList {
		phantom: std::marker::PhantomData,
		prefix: NEW_COINBASE_OUTPUT_POS_PREFIX,
	}
}

pub fn kernel_index() -> MyLinkedList<CommitPos> {
	MyLinkedList {
		phantom: std::marker::PhantomData,
		prefix: KERNEL_POS_PREFIX,
	}
}

impl<T> FooLinkedList for MyLinkedList<T>
where
	T: PosEntry,
{
	type List = LinkedList<T>;
	type Entry = ListEntry<T>;

	fn list_key(&self, commit: Commitment) -> Vec<u8> {
		to_key(self.prefix, &mut commit.as_ref().to_vec())
	}

	fn entry_key(&self, commit: Commitment, pos: u64) -> Vec<u8> {
		to_key_u64(self.prefix, &mut commit.as_ref().to_vec(), pos)
	}

	/// Pop the head of the list.
	/// Returns the output_pos.
	/// Returns None if list was empty.
	fn pop_entry(&self, batch: &Batch<'_>, commit: Commitment) -> Result<Option<T>, Error> {
		match self.get_list(batch, commit)? {
			None => Ok(None),
			Some(LinkedList::Unique { pos }) => {
				// TODO - delete the list itself.

				Ok(Some(pos))
			}
			Some(LinkedList::Multi { head, tail }) => {
				// read head from db
				// read next one
				// update next to a head if it was a middle
				// update list head
				// update list to a unique if next is a tail
				Ok(None)
			}
		}
	}

	fn push_entry(&self, batch: &Batch<'_>, commit: Commitment, new_pos: T) -> Result<(), Error> {
		match self.get_list(batch, commit)? {
			None => {
				let list = LinkedList::Unique { pos: new_pos };
				batch.db.put_ser(&self.list_key(commit), &list)?;
			}
			Some(LinkedList::Unique { pos: current_pos }) => {
				let head = ListEntry::Head {
					pos: new_pos,
					next: current_pos.pos(),
				};
				let tail = ListEntry::Tail {
					pos: current_pos,
					prev: new_pos.pos(),
				};
				let list: LinkedList<T> = LinkedList::Multi {
					head: new_pos.pos(),
					tail: current_pos.pos(),
				};
				batch
					.db
					.put_ser(&self.entry_key(commit, new_pos.pos()), &head)?;
				batch
					.db
					.put_ser(&self.entry_key(commit, current_pos.pos()), &tail)?;
				batch.db.put_ser(&self.list_key(commit), &list)?;
			}
			Some(LinkedList::Multi { head, tail }) => {
				if let Some(ListEntry::Head {
					pos: current_pos,
					next: current_next,
				}) = self.get_entry(batch, commit, head)?
				{
					let head = ListEntry::Head {
						pos: new_pos,
						next: current_pos.pos(),
					};
					let middle = ListEntry::Middle {
						pos: current_pos,
						next: current_next,
						prev: new_pos.pos(),
					};
					let list: LinkedList<T> = LinkedList::Multi {
						head: new_pos.pos(),
						tail,
					};
					batch
						.db
						.put_ser(&self.entry_key(commit, new_pos.pos()), &head)?;
					batch
						.db
						.put_ser(&self.entry_key(commit, current_pos.pos()), &middle)?;
					batch.db.put_ser(&self.list_key(commit), &list)?;
				} else {
					return Err(Error::OtherErr("expected head to be head variant".into()));
				}
			}
		}
		Ok(())
	}
}

pub trait PosEntry: Readable + Writeable + Copy {
	fn pos(&self) -> u64;
}

impl PosEntry for OutputPos {
	fn pos(&self) -> u64 {
		self.pos
	}
}

pub trait FooListEntry: Readable + Writeable {
	type Pos: PosEntry;

	fn get_pos(&self) -> Self::Pos;
}

impl<T> FooListEntry for ListEntry<T>
where
	T: PosEntry,
{
	type Pos = T;

	/// Read the common pos from the various enum variants.
	fn get_pos(&self) -> Self::Pos {
		match self {
			Self::Head { pos, .. } => *pos,
			Self::Tail { pos, .. } => *pos,
			Self::Middle { pos, .. } => *pos,
		}
	}
}

pub enum ListEntry<T> {
	Head { pos: T, next: u64 },
	Tail { pos: T, prev: u64 },
	Middle { pos: T, next: u64, prev: u64 },
}

impl<T> Writeable for ListEntry<T>
where
	T: Writeable,
{
	/// Write first byte representing the variant, followed by variant specific data.
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		match self {
			ListEntry::Head { pos, next } => {
				ListEntryVariant::Head.write(writer)?;
				pos.write(writer)?;
				writer.write_u64(*next)?;
			}
			ListEntry::Tail { pos, prev } => {
				ListEntryVariant::Tail.write(writer)?;
				pos.write(writer)?;
				writer.write_u64(*prev)?;
			}
			ListEntry::Middle { pos, next, prev } => {
				ListEntryVariant::Middle.write(writer)?;
				pos.write(writer)?;
				writer.write_u64(*next)?;
				writer.write_u64(*prev)?;
			}
		}
		Ok(())
	}
}

impl<T> Readable for ListEntry<T>
where
	T: Readable,
{
	/// Read the first byte to determine what needs to be read beyond that.
	fn read(reader: &mut dyn Reader) -> Result<ListEntry<T>, ser::Error> {
		let entry = match ListEntryVariant::read(reader)? {
			ListEntryVariant::Head => ListEntry::Head {
				pos: T::read(reader)?,
				next: reader.read_u64()?,
			},
			ListEntryVariant::Tail => ListEntry::Tail {
				pos: T::read(reader)?,
				prev: reader.read_u64()?,
			},
			ListEntryVariant::Middle => ListEntry::Middle {
				pos: T::read(reader)?,
				next: reader.read_u64()?,
				prev: reader.read_u64()?,
			},
		};
		Ok(entry)
	}
}

/// An iterator on blocks, from latest to earliest, specialized to return
/// information pertaining to block difficulty calculation (timestamp and
/// previous difficulties). Mostly used by the consensus next difficulty
/// calculation.
pub struct DifficultyIter<'a> {
	start: Hash,
	store: Option<Arc<ChainStore>>,
	batch: Option<Batch<'a>>,

	// maintain state for both the "next" header in this iteration
	// and its previous header in the chain ("next next" in the iteration)
	// so we effectively read-ahead as we iterate through the chain back
	// toward the genesis block (while maintaining current state)
	header: Option<BlockHeader>,
	prev_header: Option<BlockHeader>,
}

impl<'a> DifficultyIter<'a> {
	/// Build a new iterator using the provided chain store and starting from
	/// the provided block hash.
	pub fn from<'b>(start: Hash, store: Arc<ChainStore>) -> DifficultyIter<'b> {
		DifficultyIter {
			start,
			store: Some(store),
			batch: None,
			header: None,
			prev_header: None,
		}
	}

	/// Build a new iterator using the provided chain store batch and starting from
	/// the provided block hash.
	pub fn from_batch(start: Hash, batch: Batch<'_>) -> DifficultyIter<'_> {
		DifficultyIter {
			start,
			store: None,
			batch: Some(batch),
			header: None,
			prev_header: None,
		}
	}
}

impl<'a> Iterator for DifficultyIter<'a> {
	type Item = HeaderInfo;

	fn next(&mut self) -> Option<Self::Item> {
		// Get both header and previous_header if this is the initial iteration.
		// Otherwise move prev_header to header and get the next prev_header.
		self.header = if self.header.is_none() {
			if let Some(ref batch) = self.batch {
				batch.get_block_header(&self.start).ok()
			} else if let Some(ref store) = self.store {
				store.get_block_header(&self.start).ok()
			} else {
				None
			}
		} else {
			self.prev_header.clone()
		};

		// If we have a header we can do this iteration.
		// Otherwise we are done.
		if let Some(header) = self.header.clone() {
			if let Some(ref batch) = self.batch {
				self.prev_header = batch.get_previous_header(&header).ok();
			} else if let Some(ref store) = self.store {
				self.prev_header = store.get_previous_header(&header).ok();
			} else {
				self.prev_header = None;
			}

			let prev_difficulty = self
				.prev_header
				.clone()
				.map_or(Difficulty::zero(), |x| x.total_difficulty());
			let difficulty = header.total_difficulty() - prev_difficulty;
			let scaling = header.pow.secondary_scaling;

			Some(HeaderInfo::new(
				header.hash(),
				header.timestamp.timestamp() as u64,
				difficulty,
				scaling,
				header.pow.is_secondary(),
			))
		} else {
			None
		}
	}
}
