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

use std::{fs, path};

use failure::Context;

use grin_store as store;
use grin_store::{option_to_not_found, to_key, u64_to_key};
use keychain::{Identifier, Keychain};

use libwallet::error::{Error, ErrorKind};
use libwallet::types::{WalletBackend, WalletOutputBatch, OutputData};
use types::{WalletConfig, WalletSeed, Identifier};

pub const DB_DIR: &'static str = "wallet";

const OUTPUT_PREFIX: u8 = 'o' as u8;
const DERIV_PREFIX: u8 = 'd' as u8;

impl From<store::Error> for Error {
	fn from(error: store::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::Backend(error)),
		}
	}
}

pub struct LMDBBackend<K> {
	db: store::Store,
	config: WalletConfig,

	/// Keychain
	keychain: Option<K>,
}

impl<K> LMDBBackend<K> {
	pub fn new(config: WalletConfig, passphrase: &str) -> Result<Self, Error> {
		let db_path = path::Path::new(config.data_file_dir).join(DB_DIR);
		fs::create_dir_all(&db_path)
			.expect("Couldn't create wallet backend directory!");

		let lmdb_env = store::new_env(config.db_root.clone())
		let db = store::Store::open(db_env, DB_DIR);
		Ok(LMDBBackend {
			db,
			config: config.clone(),
			keychain: None,
		})
	}
}

impl<K> WalletBackend<K> for LMDBackend<K>
where
	K: Keychain,
{
	/// Initialise with whatever stored credentials we have
	fn open_with_credentials(&mut self) -> Result<(), libwallet::Error> {
		let wallet_seed = WalletSeed::from_file(&self.config)
			.context(libwallet::ErrorKind::CallbackImpl("Error opening wallet"))?;
		let keychain = wallet_seed.derive_keychain(&self.passphrase);
		self.keychain = Some(keychain.context(libwallet::ErrorKind::CallbackImpl(
			"Error deriving keychain",
		))?);
		// Just blow up password for now after it's been used
		self.passphrase = String::from("");
		Ok(())
	}

	/// Close wallet and remove any stored credentials (TBD)
	fn close(&mut self) -> Result<(), libwallet::Error> {
		self.keychain = None;
		Ok(())
	}

	/// Return the keychain being used
	fn keychain(&mut self) -> &mut K {
		self.keychain.as_mut().unwrap()
	}

	fn iter<'a>(&'a self) -> Box<Iterator<Item = &'a OutputData> + 'a> {
		self.db.iter(&[OUTPUT_PREFIX])
	}

	fn get(&self, id: &Identifier) -> Option<OutputData> {
		option_to_not_found(self.db.get_ser(&to_key(OUTPUT_PREFIX, &mut id.to_bytes())))
	}

	fn next_child(&self, root_key_id: keychain::Identifier) -> Result<u32, Error> {
		let mut batch = self.db.batch()?;
		// a simple counter, only one batch per db guarantees atomicity
		let deriv_key = to_key(DERIV_PREFIX, &mut root_key_id.to_bytes());
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
		root_key_id: keychain::Identifier,
		amount: u64,
		current_height: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
		select_all: bool,
	) -> Vec<OutputData> {
		unimplemented!()
	}

	fn restore(&mut self) -> Result<(), libwallet::Error> {
		libwallet::internal::restore::restore(self).context(libwallet::ErrorKind::Restore)?;
		Ok(())
	}
}
