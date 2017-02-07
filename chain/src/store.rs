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

//! Implements storage primitives required by the chain

use byteorder::{WriteBytesExt, BigEndian};

use types::*;
use core::core::hash::{Hash, Hashed};
use core::core::{Block, BlockHeader};
use grin_store;

const STORE_SUBPATH: &'static str = "chain";

const SEP: u8 = ':' as u8;

const BLOCK_HEADER_PREFIX: u8 = 'h' as u8;
const BLOCK_PREFIX: u8 = 'b' as u8;
const HEAD_PREFIX: u8 = 'H' as u8;
const HEADER_HEAD_PREFIX: u8 = 'I' as u8;
const HEADER_HEIGHT_PREFIX: u8 = '8' as u8;

/// An implementation of the ChainStore trait backed by a simple key-value
/// store.
pub struct ChainKVStore {
	db: grin_store::Store,
}

impl ChainKVStore {
	pub fn new(root_path: String) -> Result<ChainKVStore, Error> {
		let db = try!(grin_store::Store::open(format!("{}/{}", root_path, STORE_SUBPATH).as_str())
			.map_err(to_store_err));
		Ok(ChainKVStore { db: db })
	}
}

impl ChainStore for ChainKVStore {
	fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEAD_PREFIX]))
	}

	fn head_header(&self) -> Result<BlockHeader, Error> {
		let head: Tip = try!(option_to_not_found(self.db.get_ser(&vec![HEAD_PREFIX])));
		self.get_block_header(&head.last_block_h)
	}

	fn save_head(&self, t: &Tip) -> Result<(), Error> {
		self.db
			.batch()
			.put_ser(&vec![HEAD_PREFIX], t)?
			.put_ser(&vec![HEADER_HEAD_PREFIX], t)?
			.write()
			.map_err(&to_store_err)
	}

	fn get_header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEADER_HEAD_PREFIX]))
	}

	fn save_header_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![HEADER_HEAD_PREFIX], t).map_err(&to_store_err)
	}

	fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_PREFIX, &mut h.to_vec())))
	}

	fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_HEADER_PREFIX, &mut h.to_vec())))
	}

	fn save_block(&self, b: &Block) -> Result<(), Error> {
		self.db
			.batch()
			.put_ser(&to_key(BLOCK_PREFIX, &mut b.hash().to_vec())[..], b)?
			.put_ser(&to_key(BLOCK_HEADER_PREFIX, &mut b.hash().to_vec())[..],
			         &b.header)?
			.write()
			.map_err(&to_store_err)
	}

	fn save_block_header(&self, bh: &BlockHeader) -> Result<(), Error> {
		self.db
			.put_ser(&to_key(BLOCK_HEADER_PREFIX, &mut bh.hash().to_vec())[..],
			         bh)
			.map_err(&to_store_err)
	}

	fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		option_to_not_found(self.db.get_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, height)))
	}

	fn setup_height(&self, bh: &BlockHeader) -> Result<(), Error> {
		self.db.put_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, bh.height), bh).map_err(&to_store_err)?;

		let mut prev_h = bh.previous;
		let mut prev_height = bh.height - 1;
		while prev_height > 0 {
			let prev = self.get_header_by_height(prev_height)?;
			if prev.hash() != prev_h {
				let real_prev = self.get_block_header(&prev_h)?;
				self.db
					.put_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, real_prev.height),
					         &real_prev)
					.map_err(&to_store_err)?;
				prev_h = real_prev.previous;
				prev_height = real_prev.height - 1;
			} else {
				break;
			}
		}
		Ok(())
	}
}

impl From<grin_store::Error> for Error {
	fn from(e: grin_store::Error) -> Error {
		Error::StorageErr(e.to_string())
	}
}

fn to_key(prefix: u8, val: &mut Vec<u8>) -> &mut Vec<u8> {
	val.insert(0, SEP);
	val.insert(0, prefix);
	val
}

fn u64_to_key<'a>(prefix: u8, val: u64) -> Vec<u8> {
	let mut u64_vec = vec![];
	u64_vec.write_u64::<BigEndian>(val).unwrap();
	u64_vec.insert(0, SEP);
	u64_vec.insert(0, prefix);
	u64_vec
}

fn to_store_err(e: grin_store::Error) -> Error {
	Error::StorageErr(format!("{:?}", e))
}

/// unwraps the inner option by converting the none case to a not found error
fn option_to_not_found<T>(res: Result<Option<T>, grin_store::Error>) -> Result<T, Error> {
	match res {
		Ok(None) => Err(Error::NotFoundErr),
		Ok(Some(o)) => Ok(o),
		Err(e) => Err(to_store_err(e)),
	}
}
