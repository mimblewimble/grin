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

//! Temporary utility to migrate wallet data from file to a database

use keychain::{ExtKeychain, Identifier, Keychain};
use std::fs::File;
use std::io::Read;
use std::path::{Path, MAIN_SEPARATOR};
/// Migrate wallet data. Assumes current directory contains a set of wallet
/// files
use std::sync::Arc;

use error::{Error, ErrorKind};
use failure::ResultExt;

use serde_json;

use libwallet::types::WalletDetails;
use types::WalletSeed;

use libwallet::types::OutputData;
use store::{self, to_key};

const DETAIL_FILE: &'static str = "wallet.det";
const DAT_FILE: &'static str = "wallet.dat";
const SEED_FILE: &'static str = "wallet.seed";
const DB_DIR: &'static str = "db";
const OUTPUT_PREFIX: u8 = 'o' as u8;
const DERIV_PREFIX: u8 = 'd' as u8;
const CONFIRMED_HEIGHT_PREFIX: u8 = 'c' as u8;

// determine whether we have wallet files but no file wallet
pub fn needs_migrate(data_dir: &str) -> bool {
	let db_path = Path::new(data_dir).join(DB_DIR);
	let data_path = Path::new(data_dir).join(DAT_FILE);
	if !db_path.exists() && data_path.exists() {
		return true;
	}
	false
}

pub fn migrate(data_dir: &str, pwd: &str) -> Result<(), Error> {
	let data_file_path = format!("{}{}{}", data_dir, MAIN_SEPARATOR, DAT_FILE);
	let details_file_path = format!("{}{}{}", data_dir, MAIN_SEPARATOR, DETAIL_FILE);
	let seed_file_path = format!("{}{}{}", data_dir, MAIN_SEPARATOR, SEED_FILE);
	let outputs = read_outputs(&data_file_path)?;
	let details = read_details(&details_file_path)?;

	let mut file = File::open(seed_file_path).context(ErrorKind::IO)?;
	let mut buffer = String::new();
	file.read_to_string(&mut buffer).context(ErrorKind::IO)?;
	let wallet_seed = WalletSeed::from_hex(&buffer)?;
	let keychain: ExtKeychain = wallet_seed.derive_keychain(pwd)?;
	let root_key_id = keychain.root_key_id();

	//open db
	let db_path = Path::new(data_dir).join(DB_DIR);
	let lmdb_env = Arc::new(store::new_env(db_path.to_str().unwrap().to_string()));

	// open store
	let store = store::Store::open(lmdb_env, DB_DIR);
	let batch = store.batch().unwrap();

	// write
	for out in outputs {
		save_output(&batch, out.clone())?;
	}
	save_details(&batch, root_key_id, details)?;

	let res = batch.commit();
	if let Err(e) = res {
		panic!("Unable to commit db: {:?}", e);
	}

	Ok(())
}

/// save output in db
fn save_output(batch: &store::Batch, out: OutputData) -> Result<(), Error> {
	let key = to_key(OUTPUT_PREFIX, &mut out.key_id.to_bytes().to_vec());
	if let Err(e) = batch.put_ser(&key, &out) {
		Err(ErrorKind::GenericError(format!(
			"Error inserting output: {:?}",
			e
		)))?;
	}
	Ok(())
}

/// save details in db
fn save_details(
	batch: &store::Batch,
	root_key_id: Identifier,
	d: WalletDetails,
) -> Result<(), Error> {
	let deriv_key = to_key(DERIV_PREFIX, &mut root_key_id.to_bytes().to_vec());
	let height_key = to_key(
		CONFIRMED_HEIGHT_PREFIX,
		&mut root_key_id.to_bytes().to_vec(),
	);
	if let Err(e) = batch.put_ser(&deriv_key, &d.last_child_index) {
		Err(ErrorKind::GenericError(format!(
			"Error saving last_child_index: {:?}",
			e
		)))?;
	}
	if let Err(e) = batch.put_ser(&height_key, &d.last_confirmed_height) {
		Err(ErrorKind::GenericError(format!(
			"Error saving last_confirmed_height: {:?}",
			e
		)))?;
	}
	Ok(())
}

/// Read output_data vec from disk.
fn read_outputs(data_file_path: &str) -> Result<Vec<OutputData>, Error> {
	let data_file = File::open(data_file_path.clone())
		.context(ErrorKind::FileWallet(&"Could not open wallet file"))?;
	serde_json::from_reader(data_file)
		.context(ErrorKind::Format)
		.map_err(|e| e.into())
}

/// Read details file from disk
fn read_details(details_file_path: &str) -> Result<WalletDetails, Error> {
	let details_file = File::open(details_file_path.clone())
		.context(ErrorKind::FileWallet(&"Could not open wallet details file"))?;
	serde_json::from_reader(details_file)
		.context(ErrorKind::Format)
		.map_err(|e| e.into())
}

#[ignore]
#[test]
fn migrate_db() {
	let _ = migrate("test_wallet", "");
}
