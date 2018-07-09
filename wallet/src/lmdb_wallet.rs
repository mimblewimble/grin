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

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::{fs, path};

use failure::ResultExt;

use keychain::{Identifier, Keychain};
use store::{self, option_to_not_found, to_key};

use client;
use libtx::slate::Slate;
use libwallet::types::*;
use libwallet::{internal, Error, ErrorKind};
use types::{WalletConfig, WalletSeed};
use util::secp::pedersen;

pub const DB_DIR: &'static str = "wallet_data";

const OUTPUT_PREFIX: u8 = 'o' as u8;
const DERIV_PREFIX: u8 = 'd' as u8;

impl From<store::Error> for Error {
	fn from(error: store::Error) -> Error {
		Error::from(ErrorKind::Backend(format!("{:?}", error)))
	}
}

/// test to see if database files exist in the current directory. If so,
/// use a DB backend for all operations
pub fn wallet_db_exists(config: WalletConfig) -> bool {
	let db_path = path::Path::new(&config.data_file_dir).join(DB_DIR);
	db_path.exists()
}

pub struct LMDBBackend<K> {
	db: store::Store,
	config: WalletConfig,
	/// passphrase: TODO better ways of dealing with this other than storing
	passphrase: String,

	/// Keychain
	keychain: Option<K>,
}

impl<K> LMDBBackend<K> {
	pub fn new(config: WalletConfig, passphrase: &str) -> Result<Self, Error> {
		let db_path = path::Path::new(&config.data_file_dir).join(DB_DIR);
		fs::create_dir_all(&db_path).expect("Couldn't create wallet backend directory!");

		let lmdb_env = Arc::new(store::new_env(db_path.to_str().unwrap().to_string()));
		let db = store::Store::open(lmdb_env, DB_DIR);
		Ok(LMDBBackend {
			db,
			config: config.clone(),
			passphrase: String::from(passphrase),
			keychain: None,
		})
	}

	/// Just test to see if database files exist in the current directory. If
	/// so, use a DB backend for all operations
	pub fn exists(config: WalletConfig) -> bool {
		let db_path = path::Path::new(&config.data_file_dir).join(DB_DIR);
		db_path.exists()
	}
}

impl<K> WalletBackend<K> for LMDBBackend<K>
where
	K: Keychain,
{
	/// Initialise with whatever stored credentials we have
	fn open_with_credentials(&mut self) -> Result<(), Error> {
		let wallet_seed = WalletSeed::from_file(&self.config)
			.context(ErrorKind::CallbackImpl("Error opening wallet"))?;
		let keychain = wallet_seed.derive_keychain(&self.passphrase);
		self.keychain = Some(keychain.context(ErrorKind::CallbackImpl("Error deriving keychain"))?);
		// Just blow up password for now after it's been used
		self.passphrase = String::from("");
		Ok(())
	}

	/// Close wallet and remove any stored credentials (TBD)
	fn close(&mut self) -> Result<(), Error> {
		self.keychain = None;
		Ok(())
	}

	/// Return the keychain being used
	fn keychain(&mut self) -> &mut K {
		self.keychain.as_mut().unwrap()
	}

	fn get(&self, id: &Identifier) -> Result<OutputData, Error> {
		let key = to_key(OUTPUT_PREFIX, &mut id.to_bytes().to_vec());
		option_to_not_found(self.db.get_ser(&key), &format!("Key Id: {}", id)).map_err(|e| e.into())
	}

	fn iter<'a>(&'a self) -> Box<Iterator<Item = OutputData> + 'a> {
		Box::new(self.db.iter(&[OUTPUT_PREFIX]).unwrap())
	}

	fn batch<'a>(&'a mut self) -> Result<Box<WalletOutputBatch + 'a>, Error> {
		Ok(Box::new(Batch {
			store: self,
			db: RefCell::new(Some(self.db.batch()?)),
		}))
	}

	fn next_child<'a>(&mut self, root_key_id: Identifier) -> Result<u32, Error> {
		let batch = self.db.batch()?;
		// a simple counter, only one batch per db guarantees atomicity
		let deriv_key = to_key(DERIV_PREFIX, &mut root_key_id.to_bytes().to_vec());
		let deriv_idx = match batch.get_ser(&deriv_key)? {
			Some(idx) => idx,
			None => 0,
		};
		batch.put_ser(&deriv_key, &(deriv_idx + 1))?;
		batch.commit()?;
		Ok(deriv_idx + 1)
	}

	fn select_coins(
		&self,
		root_key_id: Identifier,
		amount: u64,
		current_height: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
		select_all: bool,
	) -> Vec<OutputData> {
		unimplemented!()
	}

	fn details(&mut self) -> &mut WalletDetails {
		unimplemented!()
	}

	fn restore(&mut self) -> Result<(), Error> {
		internal::restore::restore(self).context(ErrorKind::Restore)?;
		Ok(())
	}
}

/// An atomic batch in which all changes can be committed all at once or
/// discarded on error.
pub struct Batch<'a, K: 'a> {
	store: &'a LMDBBackend<K>,
	db: RefCell<Option<store::Batch<'a>>>,
}

#[allow(missing_docs)]
impl<'a, K> WalletOutputBatch for Batch<'a, K> {
	fn save(&mut self, out: OutputData) -> Result<(), Error> {
		let key = to_key(OUTPUT_PREFIX, &mut out.key_id.to_bytes().to_vec());
		self.db.borrow().as_ref().unwrap().put_ser(&key, &out)?;
		Ok(())
	}

	fn details(&mut self) -> &mut WalletDetails {
		unimplemented!()
	}

	fn get(&self, id: &Identifier) -> Result<OutputData, Error> {
		let key = to_key(OUTPUT_PREFIX, &mut id.to_bytes().to_vec());
		option_to_not_found(
			self.db.borrow().as_ref().unwrap().get_ser(&key),
			&format!("Key ID: {}", id),
		).map_err(|e| e.into())
	}

	fn iter<'b>(&'b self) -> Box<Iterator<Item = OutputData> + 'b> {
		unimplemented!();
	}

	fn delete(&mut self, id: &Identifier) -> Result<(), Error> {
		let key = to_key(OUTPUT_PREFIX, &mut id.to_bytes().to_vec());
		self.db.borrow().as_ref().unwrap().delete(&key)?;
		Ok(())
	}

	fn lock_output(&mut self, out: &mut OutputData) -> Result<(), Error> {
		out.lock();
		self.save(out.clone())
	}

	fn commit(&self) -> Result<(), Error> {
		let db = self.db.replace(None);
		db.unwrap().commit()?;
		Ok(())
	}
}

impl<K> WalletClient for LMDBBackend<K> {
	/// Return URL for check node
	fn node_url(&self) -> &str {
		&self.config.check_node_api_http_addr
	}

	/// Call the wallet API to create a coinbase transaction
	fn create_coinbase(&self, block_fees: &BlockFees) -> Result<CbData, Error> {
		let res = client::create_coinbase(self.node_url(), block_fees)
			.context(ErrorKind::WalletComms(format!("Creating Coinbase")))?;
		Ok(res)
	}

	/// Send a transaction slate to another listening wallet and return result
	fn send_tx_slate(&self, addr: &str, slate: &Slate) -> Result<Slate, Error> {
		let res = client::send_tx_slate(addr, slate)
			.context(ErrorKind::WalletComms(format!("Sending transaction")))?;
		Ok(res)
	}

	/// Posts a tranaction to a grin node
	fn post_tx(&self, tx: &TxWrapper, fluff: bool) -> Result<(), Error> {
		let res = client::post_tx(self.node_url(), tx, fluff).context(ErrorKind::Node)?;
		Ok(res)
	}

	/// retrieves the current tip from the specified grin node
	fn get_chain_height(&self) -> Result<u64, Error> {
		let res = client::get_chain_height(self.node_url()).context(ErrorKind::Node)?;
		Ok(res)
	}

	/// retrieve a list of outputs from the specified grin node
	/// need "by_height" and "by_id" variants
	fn get_outputs_from_node(
		&self,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<HashMap<pedersen::Commitment, String>, Error> {
		let res = client::get_outputs_from_node(self.node_url(), wallet_outputs)
			.context(ErrorKind::Node)?;
		Ok(res)
	}

	/// Outputs by PMMR index
	fn get_outputs_by_pmmr_index(
		&self,
		start_height: u64,
		max_outputs: u64,
	) -> Result<
		(
			u64,
			u64,
			Vec<(pedersen::Commitment, pedersen::RangeProof, bool)>,
		),
		Error,
	> {
		let res = client::get_outputs_by_pmmr_index(self.node_url(), start_height, max_outputs)
			.context(ErrorKind::Node)?;
		Ok(res)
	}
}
