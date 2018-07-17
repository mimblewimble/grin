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

//! Utilities to check the status of all the outputs we have stored in
//! the wallet storage and update them.

use failure::ResultExt;
use std::collections::HashMap;

use core::consensus::reward;
use core::core::{Output, TxKernel};
use core::{global, ser};
use keychain::{Identifier, Keychain};
use libtx::reward;
use libwallet;
use libwallet::error::{Error, ErrorKind};
use libwallet::internal::keys;
use libwallet::types::{
	BlockFees, CbData, OutputData, OutputStatus, TxLogEntry, TxLogEntryType, WalletBackend,
	WalletClient, WalletInfo,
};
use util::secp::pedersen;
use util::{self, LOGGER};

/// Retrieve all of the outputs (doesn't attempt to update from node)
pub fn retrieve_outputs<T: ?Sized, C, K>(
	wallet: &mut T,
	show_spent: bool,
) -> Result<Vec<OutputData>, Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let root_key_id = wallet.keychain().clone().root_key_id();
	// just read the wallet here, no need for a write lock
	let mut outputs = wallet
		.iter()
		.filter(|out| out.root_key_id == root_key_id)
		.filter(|out| {
			if show_spent {
				true
			} else {
				out.status != OutputStatus::Spent
			}
		})
		.collect::<Vec<_>>();
	outputs.sort_by_key(|out| out.n_child);
	Ok(outputs)
}

/// Retrieve all of the transaction entries
pub fn retrieve_txs<T: ?Sized, C, K>(wallet: &mut T) -> Result<Vec<TxLogEntry>, Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	// just read the wallet here, no need for a write lock
	let mut txs = wallet.tx_log_iter().collect::<Vec<_>>();
	txs.sort_by_key(|tx| tx.creation_ts);
	Ok(txs)
}
/// Refreshes the outputs in a wallet with the latest information
/// from a node
pub fn refresh_outputs<T: ?Sized, C, K>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let height = wallet.client().get_chain_height()?;
	refresh_output_state(wallet, height)?;
	Ok(())
}

/// build a local map of wallet outputs keyed by commit
/// and a list of outputs we want to query the node for
pub fn map_wallet_outputs<T: ?Sized, C, K>(
	wallet: &mut T,
) -> Result<HashMap<pedersen::Commitment, Identifier>, Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let mut wallet_outputs: HashMap<pedersen::Commitment, Identifier> = HashMap::new();
	let keychain = wallet.keychain().clone();
	let root_key_id = keychain.root_key_id().clone();
	let unspents = wallet
		.iter()
		.filter(|x| x.root_key_id == root_key_id && x.status != OutputStatus::Spent);
	for out in unspents {
		let commit = keychain.commit_with_key_index(out.value, out.n_child)?;
		wallet_outputs.insert(commit, out.key_id.clone());
	}
	Ok(wallet_outputs)
}

/// Apply refreshed API output data to the wallet
pub fn apply_api_outputs<T: ?Sized, C, K>(
	wallet: &mut T,
	wallet_outputs: &HashMap<pedersen::Commitment, Identifier>,
	api_outputs: &HashMap<pedersen::Commitment, String>,
	height: u64,
) -> Result<(), libwallet::Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	// now for each commit, find the output in the wallet and the corresponding
	// api output (if it exists) and refresh it in-place in the wallet.
	// Note: minimizing the time we spend holding the wallet lock.
	{
		let root_key_id = wallet.keychain().root_key_id();
		let mut details = wallet.details(root_key_id.clone())?;
		// If the server height is less than our confirmed height, don't apply
		// these changes as the chain is syncing, incorrect or forking
		if height < details.last_confirmed_height {
			warn!(
				LOGGER,
				"Not updating outputs as the height of the node's chain \
				 is less than the last reported wallet update height."
			);
			warn!(
				LOGGER,
				"Please wait for sync on node to complete or fork to resolve and try again."
			);
			return Ok(());
		}
		let mut batch = wallet.batch()?;
		for (commit, id) in wallet_outputs.iter() {
			if let Ok(mut output) = batch.get(id) {
				match api_outputs.get(&commit) {
					Some(_) => {
						// if this is a coinbase tx being confirmed, it's recordable in tx log
						if output.is_coinbase && output.status == OutputStatus::Unconfirmed {
							let log_id = batch.next_tx_log_id(root_key_id.clone())?;
							let mut t = TxLogEntry::new(TxLogEntryType::ConfirmedCoinbase, log_id);
							t.confirmed = true;
							t.amount_credited = output.value;
							t.amount_debited = 0;
							t.num_outputs = 1;
							t.update_confirmation_ts();
							output.tx_log_entry = Some(log_id);
							batch.save_tx_log_entry(t)?;
						}
						// also mark the transaction in which this output is involved as confirmed
						// note that one involved input/output confirmation SHOULD be enough
						// to reliably confirm the tx
						if !output.is_coinbase && output.status == OutputStatus::Unconfirmed {
							let tx = batch
								.tx_log_iter()
								.find(|t| Some(t.id) == output.tx_log_entry);
							if let Some(mut t) = tx {
								t.update_confirmation_ts();
								t.confirmed = true;
								batch.save_tx_log_entry(t)?;
							}
						}
						output.mark_unspent();
					}
					None => output.mark_spent(),
				};
				batch.save(output)?;
			}
		}
		{
			details.last_confirmed_height = height;
			batch.save_details(root_key_id, details)?;
		}
		batch.commit()?;
	}
	Ok(())
}

/// Builds a single api query to retrieve the latest output data from the node.
/// So we can refresh the local wallet outputs.
fn refresh_output_state<T: ?Sized, C, K>(wallet: &mut T, height: u64) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	debug!(LOGGER, "Refreshing wallet outputs");

	// build a local map of wallet outputs keyed by commit
	// and a list of outputs we want to query the node for
	let wallet_outputs = map_wallet_outputs(wallet)?;

	let wallet_output_keys = wallet_outputs.keys().map(|commit| commit.clone()).collect();

	let api_outputs = wallet.client().get_outputs_from_node(wallet_output_keys)?;
	apply_api_outputs(wallet, &wallet_outputs, &api_outputs, height)?;
	clean_old_unconfirmed(wallet, height)?;
	Ok(())
}

fn clean_old_unconfirmed<T: ?Sized, C, K>(wallet: &mut T, height: u64) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	if height < 500 {
		return Ok(());
	}
	let mut ids_to_del = vec![];
	for out in wallet.iter() {
		if out.status == OutputStatus::Unconfirmed && out.height > 0 && out.height < height - 500 {
			ids_to_del.push(out.key_id.clone())
		}
	}
	let mut batch = wallet.batch()?;
	for id in ids_to_del {
		batch.delete(&id)?;
	}
	batch.commit()?;
	Ok(())
}

/// Retrieve summary info about the wallet
/// caller should refresh first if desired
pub fn retrieve_info<T: ?Sized, C, K>(wallet: &mut T) -> Result<WalletInfo, Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let root_key_id = wallet.keychain().root_key_id();
	let current_height = wallet.details(root_key_id.clone())?.last_confirmed_height;
	let keychain = wallet.keychain().clone();
	let outputs = wallet
		.iter()
		.filter(|out| out.root_key_id == keychain.root_key_id());

	let mut unspent_total = 0;
	let mut immature_total = 0;
	let mut unconfirmed_total = 0;
	let mut locked_total = 0;
	for out in outputs {
		if out.status == OutputStatus::Unspent && out.lock_height <= current_height {
			unspent_total += out.value;
		}
		if out.status == OutputStatus::Unspent && out.lock_height > current_height {
			immature_total += out.value;
		}
		if out.status == OutputStatus::Unconfirmed && !out.is_coinbase {
			unconfirmed_total += out.value;
		}
		if out.status == OutputStatus::Locked {
			locked_total += out.value;
		}
	}

	Ok(WalletInfo {
		last_confirmed_height: current_height,
		total: unspent_total + unconfirmed_total + immature_total,
		amount_awaiting_confirmation: unconfirmed_total,
		amount_immature: immature_total,
		amount_locked: locked_total,
		amount_currently_spendable: unspent_total,
	})
}

/// Build a coinbase output and insert into wallet
pub fn build_coinbase<T: ?Sized, C, K>(
	wallet: &mut T,
	block_fees: &BlockFees,
) -> Result<CbData, Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let (out, kern, block_fees) = receive_coinbase(wallet, block_fees).context(ErrorKind::Node)?;

	let out_bin = ser::ser_vec(&out).context(ErrorKind::Node)?;

	let kern_bin = ser::ser_vec(&kern).context(ErrorKind::Node)?;

	let key_id_bin = match block_fees.key_id {
		Some(key_id) => ser::ser_vec(&key_id).context(ErrorKind::Node)?,
		None => vec![],
	};

	Ok(CbData {
		output: util::to_hex(out_bin),
		kernel: util::to_hex(kern_bin),
		key_id: util::to_hex(key_id_bin),
	})
}

//TODO: Split up the output creation and the wallet insertion
/// Build a coinbase output and the corresponding kernel
pub fn receive_coinbase<T: ?Sized, C, K>(
	wallet: &mut T,
	block_fees: &BlockFees,
) -> Result<(Output, TxKernel, BlockFees), Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let root_key_id = wallet.keychain().root_key_id();

	let height = block_fees.height;
	let lock_height = height + global::coinbase_maturity();
	let key_id = block_fees.key_id();

	let (key_id, derivation) = match key_id {
		Some(key_id) => keys::retrieve_existing_key(wallet, key_id)?,
		None => keys::next_available_key(wallet)?,
	};

	{
		// Now acquire the wallet lock and write the new output.
		let mut batch = wallet.batch()?;
		batch.save(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: reward(block_fees.fees),
			status: OutputStatus::Unconfirmed,
			height: height,
			lock_height: lock_height,
			is_coinbase: true,
			tx_log_entry: None,
		})?;
		batch.commit()?;
	}

	debug!(
		LOGGER,
		"receive_coinbase: built candidate output - {:?}, {}",
		key_id.clone(),
		derivation,
	);

	let mut block_fees = block_fees.clone();
	block_fees.key_id = Some(key_id.clone());

	debug!(LOGGER, "receive_coinbase: {:?}", block_fees);

	let (out, kern) = reward::output(
		wallet.keychain(),
		&key_id,
		block_fees.fees,
		block_fees.height,
	).unwrap();
	/* .context(ErrorKind::Keychain)?; */
	Ok((out, kern, block_fees))
}
