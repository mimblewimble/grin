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
use core::core::Block;
use grin_store;

const STORE_PATH: &'static str = ".grin/chain";

const SEP: u8 = ':' as u8;

const BLOCK_PREFIX: u8 = 'B' as u8;
const TIP_PREFIX: u8 = 'T' as u8;
const HEAD_PREFIX: u8 = 'H' as u8;

/// An implementation of the ChainStore trait backed by a simple key-value
/// store.
pub struct ChainKVStore {
	db: grin_store::Store,
}

impl ChainKVStore {
	pub fn new() -> Result<ChainKVStore, Error> {
		let db = try!(grin_store::Store::open(STORE_PATH).map_err(to_store_err));
		Ok(ChainKVStore { db: db })
	}
}

impl ChainStore for ChainKVStore {
	fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEAD_PREFIX]))
	}

	fn save_block(&self, b: &Block) -> Result<(), Error> {
		self.db.put_ser(&to_key(BLOCK_PREFIX, &mut b.hash().to_vec())[..], b).map_err(&to_store_err)
	}

	fn save_head(&self, t: &Tip) -> Result<(), Error> {
		try!(self.save_tip(t));
		self.db.put_ser(&vec![HEAD_PREFIX], t).map_err(&to_store_err)
	}

	fn save_tip(&self, t: &Tip) -> Result<(), Error> {
		let last_branch = t.lineage.last_branch();
		let mut k = vec![TIP_PREFIX, SEP];
		k.write_u32::<BigEndian>(last_branch);
		self.db.put_ser(&mut k, t).map_err(&to_store_err)
	}
}

fn to_key(prefix: u8, val: &mut Vec<u8>) -> &mut Vec<u8> {
	val.insert(0, SEP);
	val.insert(0, prefix);
	val
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
