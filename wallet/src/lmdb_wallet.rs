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
use std::sync::Arc;
use std::{fs, path};

use failure::ResultExt;
use uuid::Uuid;

use keychain::{ChildNumber, ExtKeychain, Identifier, Keychain};
use store::{self, option_to_not_found, to_key, to_key_u64};

use libwallet::types::*;
use libwallet::{internal, Error, ErrorKind};
use types::{WalletConfig, WalletSeed};
use util::secp::pedersen;

pub const DB_DIR: &'static str = "db";

const COMMITMENT_PREFIX: u8 = 'C' as u8;
const OUTPUT_PREFIX: u8 = 'o' as u8;
const DERIV_PREFIX: u8 = 'd' as u8;
const CONFIRMED_HEIGHT_PREFIX: u8 = 'c' as u8;
const PRIVATE_TX_CONTEXT_PREFIX: u8 = 'p' as u8;
const TX_LOG_ENTRY_PREFIX: u8 = 't' as u8;
const TX_LOG_ID_PREFIX: u8 = 'i' as u8;
const ACCOUNT_PATH_MAPPING_PREFIX: u8 = 'a' as u8;

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

pub struct LMDBBackend<C, K> {
	db: store::Store,
	config: WalletConfig,
	/// passphrase: TODO better ways of dealing with this other than storing
	passphrase: String,
	/// Keychain
	pub keychain: Option<K>,
	/// Parent path to use by default for output operations
	parent_key_id: Identifier,
	/// client
	client: C,
}

impl<C, K> LMDBBackend<C, K> {
	pub fn new(config: WalletConfig, passphrase: &str, client: C) -> Result<Self, Error> {
		let db_path = path::Path::new(&config.data_file_dir).join(DB_DIR);
		fs::create_dir_all(&db_path).expect("Couldn't create wallet backend directory!");

		let lmdb_env = Arc::new(store::new_env(db_path.to_str().unwrap().to_string()));
		let store = store::Store::open(lmdb_env, DB_DIR);

		// Make sure default wallet derivation path always exists
		let default_account = AcctPathMapping {
			label: "default".to_owned(),
			path: LMDBBackend::<C, K>::default_path(),
		};
		let acct_key = to_key(
			ACCOUNT_PATH_MAPPING_PREFIX,
			&mut default_account.label.as_bytes().to_vec(),
		);

		{
			let batch = store.batch()?;
			batch.put_ser(&acct_key, &default_account)?;
			batch.commit()?;
		}

		let res = LMDBBackend {
			db: store,
			config: config.clone(),
			passphrase: String::from(passphrase),
			keychain: None,
			parent_key_id: LMDBBackend::<C, K>::default_path(),
			client: client,
		};
		Ok(res)
	}

	fn default_path() -> Identifier {
		// return the default parent wallet path, corresponding to the default account
		// in the BIP32 spec. Parent is account 0 at level 2, child output identifiers
		// are all at level 3
		ExtKeychain::derive_key_id(2, 0, 0, 0, 0)
	}

	/// Just test to see if database files exist in the current directory. If
	/// so, use a DB backend for all operations
	pub fn exists(config: WalletConfig) -> bool {
		let db_path = path::Path::new(&config.data_file_dir).join(DB_DIR);
		db_path.exists()
	}
}

impl<C, K> WalletBackend<C, K> for LMDBBackend<C, K>
where
	C: WalletClient,
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

	/// Return the client being used
	fn client(&mut self) -> &mut C {
		&mut self.client
	}

	/// Set parent path by account name
	fn set_parent_key_id_by_name(&mut self, label: &str) -> Result<(), Error> {
		let label = label.to_owned();
		let res = self.acct_path_iter().find(|l| l.label == label);
		if let Some(a) = res {
			self.set_parent_key_id(a.path);
			Ok(())
		} else {
			return Err(ErrorKind::UnknownAccountLabel(label.clone()).into());
		}
	}

	/// set parent path
	fn set_parent_key_id(&mut self, id: Identifier) {
		self.parent_key_id = id;
	}

	fn parent_key_id(&mut self) -> Identifier {
		self.parent_key_id.clone()
	}

	fn get(&self, id: &Identifier) -> Result<OutputData, Error> {
		let key = to_key(OUTPUT_PREFIX, &mut id.to_bytes().to_vec());
		option_to_not_found(self.db.get_ser(&key), &format!("Key Id: {}", id)).map_err(|e| e.into())
	}

	fn get_commitment(&mut self, id: &Identifier) -> Result<pedersen::Commitment, Error> {
		let key = to_key(COMMITMENT_PREFIX, &mut id.to_bytes().to_vec());

		let res: Result<pedersen::Commitment, Error> =
			option_to_not_found(self.db.get_ser(&key), &format!("Key Id: {}", id))
				.map_err(|e| e.into());

		// "cache hit" and return the commitment
		if let Ok(commit) = res {
			Ok(commit)
		} else {
			let out = self.get(id)?;

			// Save the output data back to the db
			// which builds and saves the associated commitment.
			{
				let mut batch = self.batch()?;
				batch.save(out)?;
				batch.commit()?;
			}

			// Now retrieve the saved commitment and return it.
			option_to_not_found(self.db.get_ser(&key), &format!("Key Id: {}", id))
				.map_err(|e| e.into())
		}
	}

	fn iter<'a>(&'a self) -> Box<Iterator<Item = OutputData> + 'a> {
		Box::new(self.db.iter(&[OUTPUT_PREFIX]).unwrap())
	}

	fn get_tx_log_entry(&self, u: &Uuid) -> Result<Option<TxLogEntry>, Error> {
		let key = to_key(TX_LOG_ENTRY_PREFIX, &mut u.as_bytes().to_vec());
		self.db.get_ser(&key).map_err(|e| e.into())
	}

	fn tx_log_iter<'a>(&'a self) -> Box<Iterator<Item = TxLogEntry> + 'a> {
		Box::new(self.db.iter(&[TX_LOG_ENTRY_PREFIX]).unwrap())
	}

	fn get_private_context(&mut self, slate_id: &[u8]) -> Result<Context, Error> {
		let ctx_key = to_key(PRIVATE_TX_CONTEXT_PREFIX, &mut slate_id.to_vec());
		option_to_not_found(
			self.db.get_ser(&ctx_key),
			&format!("Slate id: {:x?}", slate_id.to_vec()),
		).map_err(|e| e.into())
	}

	fn acct_path_iter<'a>(&'a self) -> Box<Iterator<Item = AcctPathMapping> + 'a> {
		Box::new(self.db.iter(&[ACCOUNT_PATH_MAPPING_PREFIX]).unwrap())
	}

	fn get_acct_path(&self, label: String) -> Result<Option<AcctPathMapping>, Error> {
		let acct_key = to_key(ACCOUNT_PATH_MAPPING_PREFIX, &mut label.as_bytes().to_vec());
		self.db.get_ser(&acct_key).map_err(|e| e.into())
	}

	fn batch<'a>(&'a mut self) -> Result<Box<WalletOutputBatch<K> + 'a>, Error> {
		Ok(Box::new(Batch {
			_store: self,
			db: RefCell::new(Some(self.db.batch()?)),
			keychain: self.keychain.clone(),
		}))
	}

	fn next_child<'a>(&mut self) -> Result<Identifier, Error> {
		let parent_key_id = self.parent_key_id.clone();
		let mut deriv_idx = {
			let batch = self.db.batch()?;
			let deriv_key = to_key(DERIV_PREFIX, &mut self.parent_key_id.to_bytes().to_vec());
			match batch.get_ser(&deriv_key)? {
				Some(idx) => idx,
				None => 0,
			}
		};
		let mut return_path = self.parent_key_id.to_path();
		return_path.depth = return_path.depth + 1;
		return_path.path[return_path.depth as usize - 1] = ChildNumber::from(deriv_idx);
		deriv_idx = deriv_idx + 1;
		let mut batch = self.batch()?;
		batch.save_child_index(&parent_key_id, deriv_idx)?;
		batch.commit()?;
		Ok(Identifier::from_path(&return_path))
	}

	fn last_confirmed_height<'a>(&mut self) -> Result<u64, Error> {
		let batch = self.db.batch()?;
		let height_key = to_key(
			CONFIRMED_HEIGHT_PREFIX,
			&mut self.parent_key_id.to_bytes().to_vec(),
		);
		let last_confirmed_height = match batch.get_ser(&height_key)? {
			Some(h) => h,
			None => 0,
		};
		Ok(last_confirmed_height)
	}

	fn restore(&mut self) -> Result<(), Error> {
		internal::restore::restore(self).context(ErrorKind::Restore)?;
		Ok(())
	}
}

/// An atomic batch in which all changes can be committed all at once or
/// discarded on error.
pub struct Batch<'a, C: 'a, K: 'a>
where
	C: WalletClient,
	K: Keychain,
{
	_store: &'a LMDBBackend<C, K>,
	db: RefCell<Option<store::Batch<'a>>>,
	/// Keychain
	keychain: Option<K>,
}

#[allow(missing_docs)]
impl<'a, C, K> WalletOutputBatch<K> for Batch<'a, C, K>
where
	C: WalletClient,
	K: Keychain,
{
	fn keychain(&mut self) -> &mut K {
		self.keychain.as_mut().unwrap()
	}

	fn save(&mut self, out: OutputData) -> Result<(), Error> {
		// Save the output data to the db.
		{
			let key = to_key(OUTPUT_PREFIX, &mut out.key_id.to_bytes().to_vec());
			self.db.borrow().as_ref().unwrap().put_ser(&key, &out)?;
		}

		// Save the associated output commitment.
		{
			let key = to_key(COMMITMENT_PREFIX, &mut out.key_id.to_bytes().to_vec());
			let commit = self.keychain().commit(out.value, &out.key_id)?;
			self.db.borrow().as_ref().unwrap().put_ser(&key, &commit)?;
		}

		Ok(())
	}

	fn get(&self, id: &Identifier) -> Result<OutputData, Error> {
		let key = to_key(OUTPUT_PREFIX, &mut id.to_bytes().to_vec());
		option_to_not_found(
			self.db.borrow().as_ref().unwrap().get_ser(&key),
			&format!("Key ID: {}", id),
		).map_err(|e| e.into())
	}

	fn iter(&self) -> Box<Iterator<Item = OutputData>> {
		Box::new(
			self.db
				.borrow()
				.as_ref()
				.unwrap()
				.iter(&[OUTPUT_PREFIX])
				.unwrap(),
		)
	}

	fn delete(&mut self, id: &Identifier) -> Result<(), Error> {
		// Delete the output data.
		{
			let key = to_key(OUTPUT_PREFIX, &mut id.to_bytes().to_vec());
			let _ = self.db.borrow().as_ref().unwrap().delete(&key);
		}

		// Delete the associated output commitment.
		{
			let key = to_key(COMMITMENT_PREFIX, &mut id.to_bytes().to_vec());
			let _ = self.db.borrow().as_ref().unwrap().delete(&key);
		}

		Ok(())
	}

	fn next_tx_log_id(&mut self, parent_key_id: &Identifier) -> Result<u32, Error> {
		let tx_id_key = to_key(TX_LOG_ID_PREFIX, &mut parent_key_id.to_bytes().to_vec());
		let last_tx_log_id = match self.db.borrow().as_ref().unwrap().get_ser(&tx_id_key)? {
			Some(t) => t,
			None => 0,
		};
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&tx_id_key, &(last_tx_log_id + 1))?;
		Ok(last_tx_log_id)
	}

	fn tx_log_iter(&self) -> Box<Iterator<Item = TxLogEntry>> {
		Box::new(
			self.db
				.borrow()
				.as_ref()
				.unwrap()
				.iter(&[TX_LOG_ENTRY_PREFIX])
				.unwrap(),
		)
	}

	fn save_last_confirmed_height(
		&mut self,
		parent_key_id: &Identifier,
		height: u64,
	) -> Result<(), Error> {
		let height_key = to_key(
			CONFIRMED_HEIGHT_PREFIX,
			&mut parent_key_id.to_bytes().to_vec(),
		);
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&height_key, &height)?;
		Ok(())
	}

	fn save_child_index(&mut self, parent_id: &Identifier, child_n: u32) -> Result<(), Error> {
		let deriv_key = to_key(DERIV_PREFIX, &mut parent_id.to_bytes().to_vec());
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&deriv_key, &child_n)?;
		Ok(())
	}

	fn save_tx_log_entry(&self, t: TxLogEntry, parent_id: &Identifier) -> Result<(), Error> {
		let tx_log_key = to_key_u64(
			TX_LOG_ENTRY_PREFIX,
			&mut parent_id.to_bytes().to_vec(),
			t.id as u64,
		);
		self.db.borrow().as_ref().unwrap().put_ser(&tx_log_key, &t)?;
		Ok(())
	}

	fn save_acct_path(&mut self, mapping: AcctPathMapping) -> Result<(), Error> {
		let acct_key = to_key(
			ACCOUNT_PATH_MAPPING_PREFIX,
			&mut mapping.label.as_bytes().to_vec(),
		);
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&acct_key, &mapping)?;
		Ok(())
	}

	fn acct_path_iter(&self) -> Box<Iterator<Item = AcctPathMapping>> {
		Box::new(
			self.db
				.borrow()
				.as_ref()
				.unwrap()
				.iter(&[ACCOUNT_PATH_MAPPING_PREFIX])
				.unwrap(),
		)
	}

	fn lock_output(&mut self, out: &mut OutputData) -> Result<(), Error> {
		out.lock();
		self.save(out.clone())
	}

	fn save_private_context(&mut self, slate_id: &[u8], ctx: &Context) -> Result<(), Error> {
		let ctx_key = to_key(PRIVATE_TX_CONTEXT_PREFIX, &mut slate_id.to_vec());
		self.db.borrow().as_ref().unwrap().put_ser(&ctx_key, &ctx)?;
		Ok(())
	}

	fn delete_private_context(&mut self, slate_id: &[u8]) -> Result<(), Error> {
		let ctx_key = to_key(PRIVATE_TX_CONTEXT_PREFIX, &mut slate_id.to_vec());
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.delete(&ctx_key)
			.map_err(|e| e.into())
	}

	fn commit(&self) -> Result<(), Error> {
		let db = self.db.replace(None);
		db.unwrap().commit()?;
		Ok(())
	}
}
