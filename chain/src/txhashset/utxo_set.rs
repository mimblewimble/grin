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

//! Hash only MMR over UTXO set (spent outputs are zero'd out).

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use core::core::hash::Hash;
use core::core::pmmr::{self, DBPMMR, ReadonlyPMMR};
use core::core::{Block, Input, Output, OutputIdentifier, Transaction, UTXOEntry};
use error::{Error, ErrorKind};
use grin_store::pmmr::{HashOnlyMMRBackend, PMMRBackend};
use txhashset::utxo_view::UTXOView;

const UTXOSET_SUBDIR: &'static str = "utxoset";
const UTXOSET_UTXO_SUBDIR: &'static str = "utxo";

struct HashOnlyMMRHandle {
	backend: HashOnlyMMRBackend,
	last_pos: u64,
}

impl HashOnlyMMRHandle {
	fn new(root_dir: &str, sub_dir: &str, file_name: &str) -> Result<HashOnlyMMRHandle, Error> {
		let path = Path::new(root_dir).join(sub_dir).join(file_name);
		fs::create_dir_all(path.clone())?;
		let backend = HashOnlyMMRBackend::new(path.to_str().unwrap())?;
		let last_pos = backend.unpruned_size()?;
		Ok(HashOnlyMMRHandle { backend, last_pos })
	}
}

/// MMR over the UTXO set.
pub struct UTXOSet {
	pmmr_h: HashOnlyMMRHandle,
}

impl UTXOSet {
	pub fn open(
		root_dir: String,
	) -> Result<UTXOSet, Error> {
		Ok(UTXOSet {
			pmmr_h: HashOnlyMMRHandle::new(
				&root_dir,
				UTXOSET_SUBDIR,
				UTXOSET_UTXO_SUBDIR,
			)?,
		})
	}
}

pub fn extending<'a, F, T>(utxo_set: &'a mut UTXOSet, inner: F) -> Result<T, Error>
where
	F: FnOnce(&mut Extension) -> Result<T, Error>,
{
	let res = {
		let mut extension = Extension::new(utxo_set);
		inner(&mut extension)
	};

	utxo_set.pmmr_h.backend.discard();

	res
}

pub struct Extension<'a> {
	pmmr: DBPMMR<'a, UTXOEntry, HashOnlyMMRBackend>,
}

impl<'a> Extension<'a> {
	fn new(utxo: &'a mut UTXOSet) -> Extension<'a> {
		Extension {
			pmmr: DBPMMR::at(
				&mut utxo.pmmr_h.backend,
				utxo.pmmr_h.last_pos,
			)
		}
	}

	pub fn root(&self) -> Hash {
		self.pmmr.root()
	}

	pub fn truncate(&mut self) -> Result<(), Error> {
		debug!("Truncating utxo_set extension.");
		self.pmmr.rewind(0).map_err(&ErrorKind::TxHashSetErr)?;
		Ok(())
	}

	fn apply_entry(&mut self, entry: &UTXOEntry) -> Result<(u64), Error> {
		let pos = self
			.pmmr
			.push(entry)
			.map_err(&ErrorKind::TxHashSetErr)?;
		Ok(pos)
	}

	pub fn rebuild(&mut self, utxo_view: &UTXOView) -> Result<(), Error> {
		debug!("*** rebuild: rebuilding the utxo_set (rewinding aka truncating to pos 0 first).");
		// Trucate the extension (back to pos 0).
		self.truncate()?;

		for n in 1..utxo_view.size() + 1 {
			if pmmr::is_leaf(n) {
				if let Some(out) = utxo_view.get(n) {
					self.apply_entry(&UTXOEntry::Unspent(out))?;
				} else {
					self.apply_entry(&UTXOEntry::Spent)?;
				}
			}
		}
		Ok(())
	}
}
