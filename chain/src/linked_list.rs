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
use crate::store::{
	Batch, KERNEL_POS_PREFIX, NEW_COINBASE_OUTPUT_POS_PREFIX, NEW_PLAIN_OUTPUT_POS_PREFIX,
};
use crate::types::{CommitPos, OutputPos, Tip};
use crate::util::secp::pedersen::Commitment;
use croaring::Bitmap;
use enum_primitive::FromPrimitive;
use grin_store as store;
use grin_store::{option_to_not_found, to_key, to_key_u64, Error, SerIterator};
use std::convert::TryInto;
use std::marker::PhantomData;
use std::sync::Arc;

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
	phantom: PhantomData<*const T>,
	prefix: u8,
}

pub fn output_plain_index() -> MyLinkedList<OutputPos> {
	MyLinkedList {
		phantom: PhantomData,
		prefix: NEW_PLAIN_OUTPUT_POS_PREFIX,
	}
}

pub fn output_coinbase_index() -> MyLinkedList<OutputPos> {
	MyLinkedList {
		phantom: PhantomData,
		prefix: NEW_COINBASE_OUTPUT_POS_PREFIX,
	}
}

pub fn kernel_index() -> MyLinkedList<CommitPos> {
	MyLinkedList {
		phantom: PhantomData,
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
