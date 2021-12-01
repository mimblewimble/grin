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

//! Implements "linked list" storage primitive for lmdb index supporting multiple entries.

use crate::core::ser::{self, Readable, Reader, Writeable, Writer};
use crate::store::Batch;
use crate::types::CommitPos;
use crate::util::secp::pedersen::Commitment;
use enum_primitive::FromPrimitive;
use grin_store as store;
use std::marker::PhantomData;
use store::{to_key, to_key_u64, Error};

enum_from_primitive! {
	#[derive(Copy, Clone, Debug, PartialEq)]
	enum ListWrapperVariant {
		Single = 0,
		Multi = 1,
	}
}

impl Writeable for ListWrapperVariant {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(*self as u8)
	}
}

impl Readable for ListWrapperVariant {
	fn read<R: Reader>(reader: &mut R) -> Result<ListWrapperVariant, ser::Error> {
		ListWrapperVariant::from_u8(reader.read_u8()?).ok_or(ser::Error::CorruptedData)
	}
}

enum_from_primitive! {
	#[derive(Copy, Clone, Debug, PartialEq)]
	enum ListEntryVariant {
		// Start at 2 here to differentiate from ListWrapperVariant above.
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
	fn read<R: Reader>(reader: &mut R) -> Result<ListEntryVariant, ser::Error> {
		ListEntryVariant::from_u8(reader.read_u8()?).ok_or(ser::Error::CorruptedData)
	}
}

/// Index supporting a list of (duplicate) entries per commitment.
/// Each entry will be at a unique MMR pos.
pub trait ListIndex {
	/// List type
	type List: Readable + Writeable;

	/// List entry type
	type Entry: ListIndexEntry;

	/// Construct a key for the list.
	fn list_key(&self, commit: Commitment) -> Vec<u8>;

	/// Construct a key for an individual entry in the list.
	fn entry_key(&self, commit: Commitment, pos: u64) -> Vec<u8>;

	/// Returns either a "Single" with embedded "pos" or a "list" with "head" and "tail".
	/// Key is "prefix|commit".
	/// Note the key for an individual entry in the list is "prefix|commit|pos".
	fn get_list(&self, batch: &Batch<'_>, commit: Commitment) -> Result<Option<Self::List>, Error> {
		batch.db.get_ser(&self.list_key(commit), None)
	}

	/// Returns one of "head", "tail" or "middle" entry variants.
	/// Key is "prefix|commit|pos".
	fn get_entry(
		&self,
		batch: &Batch<'_>,
		commit: Commitment,
		pos: u64,
	) -> Result<Option<Self::Entry>, Error> {
		batch.db.get_ser(&self.entry_key(commit, pos), None)
	}

	/// Peek the head of the list for the specified commitment.
	fn peek_pos(
		&self,
		batch: &Batch<'_>,
		commit: Commitment,
	) -> Result<Option<<Self::Entry as ListIndexEntry>::Pos>, Error>;

	/// Push a pos onto the list for the specified commitment.
	fn push_pos(
		&self,
		batch: &Batch<'_>,
		commit: Commitment,
		new_pos: <Self::Entry as ListIndexEntry>::Pos,
	) -> Result<(), Error>;

	/// Pop a pos off the list for the specified commitment.
	fn pop_pos(
		&self,
		batch: &Batch<'_>,
		commit: Commitment,
	) -> Result<Option<<Self::Entry as ListIndexEntry>::Pos>, Error>;
}

/// Supports "rewind" given the provided commit and a pos to rewind back to.
pub trait RewindableListIndex {
	/// Rewind the index for the given commitment to the specified position.
	fn rewind(&self, batch: &Batch<'_>, commit: Commitment, rewind_pos: u64) -> Result<(), Error>;
}

/// A pruneable list index supports pruning of old data from the index lists.
/// This allows us to efficiently maintain an index of "recent" kernel data.
/// We can maintain a window of 2 weeks of recent data, discarding anything older than this.
pub trait PruneableListIndex: ListIndex {
	/// Clear all data from the index.
	/// Used when rebuilding the index.
	fn clear(&self, batch: &Batch<'_>) -> Result<(), Error>;

	/// Prune old data.
	fn prune(&self, batch: &Batch<'_>, commit: Commitment, cutoff_pos: u64) -> Result<(), Error>;

	/// Pop a pos off the back of the list (used for pruning old data).
	fn pop_pos_back(
		&self,
		batch: &Batch<'_>,
		commit: Commitment,
	) -> Result<Option<<Self::Entry as ListIndexEntry>::Pos>, Error>;
}

/// Wrapper for the list to handle either `Single` or `Multi` entries.
/// Optimized for the common case where we have a single entry in the list.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ListWrapper<T> {
	/// List with a single entry.
	/// Allows direct access to the pos.
	Single {
		/// The MMR pos where this single entry is located.
		pos: T,
	},
	/// List with multiple entries.
	/// Maintains head and tail of the underlying linked list.
	Multi {
		/// Head of the linked list.
		head: u64,
		/// Tail of the linked list.
		tail: u64,
	},
}

impl<T> Writeable for ListWrapper<T>
where
	T: Writeable,
{
	/// Write first byte representing the variant, followed by variant specific data.
	/// "Single" is optimized with embedded "pos".
	/// "Multi" has references to "head" and "tail".
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		match self {
			ListWrapper::Single { pos } => {
				ListWrapperVariant::Single.write(writer)?;
				pos.write(writer)?;
			}
			ListWrapper::Multi { head, tail } => {
				ListWrapperVariant::Multi.write(writer)?;
				writer.write_u64(*head)?;
				writer.write_u64(*tail)?;
			}
		}
		Ok(())
	}
}

impl<T> Readable for ListWrapper<T>
where
	T: Readable,
{
	/// Read the first byte to determine what needs to be read beyond that.
	fn read<R: Reader>(reader: &mut R) -> Result<ListWrapper<T>, ser::Error> {
		let entry = match ListWrapperVariant::read(reader)? {
			ListWrapperVariant::Single => ListWrapper::Single {
				pos: T::read(reader)?,
			},
			ListWrapperVariant::Multi => ListWrapper::Multi {
				head: reader.read_u64()?,
				tail: reader.read_u64()?,
			},
		};
		Ok(entry)
	}
}

/// Index supporting multiple duplicate entries.
pub struct MultiIndex<T> {
	phantom: PhantomData<*const T>,
	list_prefix: u8,
	entry_prefix: u8,
}

impl<T> MultiIndex<T> {
	/// Initialize a new multi index with the specified list and entry prefixes.
	pub fn init(list_prefix: u8, entry_prefix: u8) -> MultiIndex<T> {
		MultiIndex {
			phantom: PhantomData,
			list_prefix,
			entry_prefix,
		}
	}
}

impl<T> ListIndex for MultiIndex<T>
where
	T: PosEntry,
{
	type List = ListWrapper<T>;
	type Entry = ListEntry<T>;

	fn list_key(&self, commit: Commitment) -> Vec<u8> {
		to_key(self.list_prefix, &mut commit.as_ref().to_vec())
	}

	fn entry_key(&self, commit: Commitment, pos: u64) -> Vec<u8> {
		to_key_u64(self.entry_prefix, &mut commit.as_ref().to_vec(), pos)
	}

	fn peek_pos(&self, batch: &Batch<'_>, commit: Commitment) -> Result<Option<T>, Error> {
		match self.get_list(batch, commit)? {
			None => Ok(None),
			Some(ListWrapper::Single { pos }) => Ok(Some(pos)),
			Some(ListWrapper::Multi { head, .. }) => {
				if let Some(ListEntry::Head { pos, .. }) = self.get_entry(batch, commit, head)? {
					Ok(Some(pos))
				} else {
					Err(Error::OtherErr("expected head to be head variant".into()))
				}
			}
		}
	}

	fn push_pos(&self, batch: &Batch<'_>, commit: Commitment, new_pos: T) -> Result<(), Error> {
		match self.get_list(batch, commit)? {
			None => {
				let list = ListWrapper::Single { pos: new_pos };
				batch.db.put_ser(&self.list_key(commit), &list)?;
			}
			Some(ListWrapper::Single { pos: current_pos }) => {
				if new_pos.pos() <= current_pos.pos() {
					return Err(Error::OtherErr("pos must be increasing".into()));
				}

				let head = ListEntry::Head {
					pos: new_pos,
					next: current_pos.pos(),
				};
				let tail = ListEntry::Tail {
					pos: current_pos,
					prev: new_pos.pos(),
				};
				let list: ListWrapper<T> = ListWrapper::Multi {
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
			Some(ListWrapper::Multi { head, tail }) => {
				if new_pos.pos() <= head {
					return Err(Error::OtherErr("pos must be increasing".into()));
				}

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
					let list: ListWrapper<T> = ListWrapper::Multi {
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

	/// Pop the head of the list.
	/// Returns the output_pos.
	/// Returns None if list was empty.
	fn pop_pos(&self, batch: &Batch<'_>, commit: Commitment) -> Result<Option<T>, Error> {
		match self.get_list(batch, commit)? {
			None => Ok(None),
			Some(ListWrapper::Single { pos }) => {
				batch.delete(&self.list_key(commit))?;
				Ok(Some(pos))
			}
			Some(ListWrapper::Multi { head, tail }) => {
				if let Some(ListEntry::Head {
					pos: current_pos,
					next: current_next,
				}) = self.get_entry(batch, commit, head)?
				{
					match self.get_entry(batch, commit, current_next)? {
						Some(ListEntry::Middle { pos, next, .. }) => {
							let head = ListEntry::Head { pos, next };
							let list: ListWrapper<T> = ListWrapper::Multi {
								head: pos.pos(),
								tail,
							};
							batch.delete(&self.entry_key(commit, current_pos.pos()))?;
							batch
								.db
								.put_ser(&self.entry_key(commit, pos.pos()), &head)?;
							batch.db.put_ser(&self.list_key(commit), &list)?;
							Ok(Some(current_pos))
						}
						Some(ListEntry::Tail { pos, .. }) => {
							let list = ListWrapper::Single { pos };
							batch.delete(&self.entry_key(commit, current_pos.pos()))?;
							batch.db.put_ser(&self.list_key(commit), &list)?;
							Ok(Some(current_pos))
						}
						Some(_) => Err(Error::OtherErr("next was unexpected".into())),
						None => Err(Error::OtherErr("next missing".into())),
					}
				} else {
					Err(Error::OtherErr("expected head to be head variant".into()))
				}
			}
		}
	}
}

/// List index that supports rewind.
impl<T: PosEntry> RewindableListIndex for MultiIndex<T> {
	fn rewind(&self, batch: &Batch<'_>, commit: Commitment, rewind_pos: u64) -> Result<(), Error> {
		while self
			.peek_pos(batch, commit)?
			.map(|x| x.pos() > rewind_pos)
			.unwrap_or(false)
		{
			self.pop_pos(batch, commit)?;
		}
		Ok(())
	}
}

impl<T: PosEntry> PruneableListIndex for MultiIndex<T> {
	fn clear(&self, batch: &Batch<'_>) -> Result<(), Error> {
		let mut list_count = 0;
		let mut entry_count = 0;
		let prefix = to_key(self.list_prefix, "");
		for key in batch.db.iter(&prefix, |k, _| Ok(k.to_vec()))? {
			let _ = batch.delete(&key);
			list_count += 1;
		}
		let prefix = to_key(self.entry_prefix, "");
		for key in batch.db.iter(&prefix, |k, _| Ok(k.to_vec()))? {
			let _ = batch.delete(&key);
			entry_count += 1;
		}
		debug!(
			"clear: lists deleted: {}, entries deleted: {}",
			list_count, entry_count
		);
		Ok(())
	}

	/// Pruning will be more performant than full rebuild but not yet necessary.
	fn prune(
		&self,
		_batch: &Batch<'_>,
		_commit: Commitment,
		_cutoff_pos: u64,
	) -> Result<(), Error> {
		unimplemented!(
			"we currently rebuild index on startup/compaction, pruning not yet implemented"
		);
	}

	/// Pop off the back/tail of the linked list.
	/// Used when pruning old data.
	fn pop_pos_back(&self, batch: &Batch<'_>, commit: Commitment) -> Result<Option<T>, Error> {
		match self.get_list(batch, commit)? {
			None => Ok(None),
			Some(ListWrapper::Single { pos }) => {
				batch.delete(&self.list_key(commit))?;
				Ok(Some(pos))
			}
			Some(ListWrapper::Multi { head, tail }) => {
				if let Some(ListEntry::Tail {
					pos: current_pos,
					prev: current_prev,
				}) = self.get_entry(batch, commit, tail)?
				{
					match self.get_entry(batch, commit, current_prev)? {
						Some(ListEntry::Middle { pos, prev, .. }) => {
							let tail = ListEntry::Tail { pos, prev };
							let list: ListWrapper<T> = ListWrapper::Multi {
								head,
								tail: pos.pos(),
							};
							batch.delete(&self.entry_key(commit, current_pos.pos()))?;
							batch
								.db
								.put_ser(&self.entry_key(commit, pos.pos()), &tail)?;
							batch.db.put_ser(&self.list_key(commit), &list)?;
							Ok(Some(current_pos))
						}
						Some(ListEntry::Head { pos, .. }) => {
							let list = ListWrapper::Single { pos };
							batch.delete(&self.entry_key(commit, current_pos.pos()))?;
							batch.db.put_ser(&self.list_key(commit), &list)?;
							Ok(Some(current_pos))
						}
						Some(_) => Err(Error::OtherErr("prev was unexpected".into())),
						None => Err(Error::OtherErr("prev missing".into())),
					}
				} else {
					Err(Error::OtherErr("expected tail to be tail variant".into()))
				}
			}
		}
	}
}

/// Something that tracks pos (in an MMR).
pub trait PosEntry: Readable + Writeable + Copy {
	/// Accessor for the underlying (MMR) pos.
	fn pos(&self) -> u64;
}

impl PosEntry for CommitPos {
	fn pos(&self) -> u64 {
		self.pos
	}
}

/// Entry maintained in the list index.
pub trait ListIndexEntry: Readable + Writeable {
	/// Type of the underlying pos indexed in the list.
	type Pos: PosEntry;

	/// Accessor for the underlying pos.
	fn get_pos(&self) -> Self::Pos;
}

impl<T> ListIndexEntry for ListEntry<T>
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

/// Head|Middle|Tail variants for the linked list entries.
pub enum ListEntry<T> {
	/// Head of ther list.
	Head {
		/// The thing in the list.
		pos: T,
		/// The next entry in the list.
		next: u64,
	},
	/// Tail of the list.
	Tail {
		/// The thing in the list.
		pos: T,
		/// The previous entry in the list.
		prev: u64,
	},
	/// An entry in the middle of the list.
	Middle {
		/// The thing in the list.
		pos: T,
		/// The next entry in the list.
		next: u64,
		/// The previous entry in the list.
		prev: u64,
	},
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
	fn read<R: Reader>(reader: &mut R) -> Result<ListEntry<T>, ser::Error> {
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
