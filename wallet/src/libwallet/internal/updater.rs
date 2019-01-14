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
use uuid::Uuid;

use crate::core::consensus::reward;
use crate::core::core::{Output, TxKernel};
use crate::core::libtx::reward;
use crate::core::{global, ser};
use crate::keychain::{Identifier, Keychain};
use crate::libwallet;
use crate::libwallet::error::{Error, ErrorKind};
use crate::libwallet::internal::keys;
use crate::libwallet::types::{
	BlockFees, CbData, NodeClient, OutputData, OutputStatus, TxLogEntry, TxLogEntryType,
	WalletBackend, WalletInfo,
};
use crate::util;
use crate::util::secp::pedersen;

/// Retrieve all of the outputs (doesn't attempt to update from node)
pub fn retrieve_outputs<T: ?Sized, C, K>(
	wallet: &mut T,
	show_spent: bool,
	tx_id: Option<u32>,
	parent_key_id: Option<&Identifier>,
) -> Result<Vec<(OutputData, pedersen::Commitment)>, Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	// just read the wallet here, no need for a write lock
	let mut outputs = wallet
		.iter()
		.filter(|out| show_spent || out.status != OutputStatus::Spent)
		.collect::<Vec<_>>();

	// only include outputs with a given tx_id if provided
	if let Some(id) = tx_id {
		outputs = outputs
			.into_iter()
			.filter(|out| out.tx_log_entry == Some(id))
			.collect::<Vec<_>>();
	}

	if let Some(k) = parent_key_id {
		outputs = outputs
			.iter()
			.filter(|o| o.root_key_id == *k)
			.map(|o| o.clone())
			.collect();
	}

	outputs.sort_by_key(|out| out.n_child);
	let keychain = wallet.keychain().clone();

	let res = outputs
		.into_iter()
		.map(|out| {
			let commit = match out.commit.clone() {
				Some(c) => pedersen::Commitment::from_vec(util::from_hex(c).unwrap()),
				None => keychain.commit(out.value, &out.key_id).unwrap(),
			};
			(out, commit)
		})
		.collect();
	Ok(res)
}

/// Retrieve all of the transaction entries, or a particular entry
/// if `parent_key_id` is set, only return entries from that key
pub fn retrieve_txs<T: ?Sized, C, K>(
	wallet: &mut T,
	tx_id: Option<u32>,
	tx_slate_id: Option<Uuid>,
	parent_key_id: Option<&Identifier>,
	outstanding_only: bool,
) -> Result<Vec<TxLogEntry>, Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let mut txs: Vec<TxLogEntry> = wallet
		.tx_log_iter()
		.filter(|tx_entry| {
			let f_pk = match parent_key_id {
				Some(k) => tx_entry.parent_key_id == *k,
				None => true,
			};
			let f_tx_id = match tx_id {
				Some(i) => tx_entry.id == i,
				None => true,
			};
			let f_txs = match tx_slate_id {
				Some(t) => tx_entry.tx_slate_id == Some(t),
				None => true,
			};
			let f_outstanding = match outstanding_only {
				true => {
					!tx_entry.confirmed
						&& (tx_entry.tx_type == TxLogEntryType::TxReceived
							|| tx_entry.tx_type == TxLogEntryType::TxSent)
				}
				false => true,
			};
			f_pk && f_tx_id && f_txs && f_outstanding
		})
		.collect();
	txs.sort_by_key(|tx| tx.creation_ts);
	Ok(txs)
}

/// Refreshes the outputs in a wallet with the latest information
/// from a node
pub fn refresh_outputs<T: ?Sized, C, K>(
	wallet: &mut T,
	parent_key_id: &Identifier,
	update_all: bool,
) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let height = wallet.w2n_client().get_chain_height()?;
	refresh_output_state(wallet, height, parent_key_id, update_all)?;
	Ok(())
}

/// build a local map of wallet outputs keyed by commit
/// and a list of outputs we want to query the node for
pub fn map_wallet_outputs<T: ?Sized, C, K>(
	wallet: &mut T,
	parent_key_id: &Identifier,
	update_all: bool,
) -> Result<HashMap<pedersen::Commitment, (Identifier, Option<u64>)>, Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let mut wallet_outputs: HashMap<pedersen::Commitment, (Identifier, Option<u64>)> =
		HashMap::new();
	let keychain = wallet.keychain().clone();
	let unspents: Vec<OutputData> = wallet
		.iter()
		.filter(|x| x.root_key_id == *parent_key_id && x.status != OutputStatus::Spent)
		.collect();

	let tx_entries = retrieve_txs(wallet, None, None, Some(&parent_key_id), true)?;

	// Only select outputs that are actually involved in an outstanding transaction
	let unspents: Vec<OutputData> = match update_all {
		false => unspents
			.into_iter()
			.filter(|x| match x.tx_log_entry.as_ref() {
				Some(t) => {
					if let Some(_) = tx_entries.iter().find(|&te| te.id == *t) {
						true
					} else {
						false
					}
				}
				None => true,
			})
			.collect(),
		true => unspents,
	};

	for out in unspents {
		let commit = match out.commit.clone() {
			Some(c) => pedersen::Commitment::from_vec(util::from_hex(c).unwrap()),
			None => keychain.commit(out.value, &out.key_id).unwrap(),
		};
		wallet_outputs.insert(commit, (out.key_id.clone(), out.mmr_index));
	}
	Ok(wallet_outputs)
}

/// Cancel transaction and associated outputs
pub fn cancel_tx_and_outputs<T: ?Sized, C, K>(
	wallet: &mut T,
	tx: TxLogEntry,
	outputs: Vec<OutputData>,
	parent_key_id: &Identifier,
) -> Result<(), libwallet::Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let mut batch = wallet.batch()?;

	for mut o in outputs {
		// unlock locked outputs
		if o.status == OutputStatus::Unconfirmed {
			batch.delete(&o.key_id, &o.mmr_index)?;
		}
		if o.status == OutputStatus::Locked {
			o.status = OutputStatus::Unspent;
			batch.save(o)?;
		}
	}
	let mut tx = tx.clone();
	if tx.tx_type == TxLogEntryType::TxSent {
		tx.tx_type = TxLogEntryType::TxSentCancelled;
	}
	if tx.tx_type == TxLogEntryType::TxReceived {
		tx.tx_type = TxLogEntryType::TxReceivedCancelled;
	}
	batch.save_tx_log_entry(tx, parent_key_id)?;
	batch.commit()?;
	Ok(())
}

/// Apply refreshed API output data to the wallet
pub fn apply_api_outputs<T: ?Sized, C, K>(
	wallet: &mut T,
	wallet_outputs: &HashMap<pedersen::Commitment, (Identifier, Option<u64>)>,
	api_outputs: &HashMap<pedersen::Commitment, (String, u64, u64)>,
	height: u64,
	parent_key_id: &Identifier,
) -> Result<(), libwallet::Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	// now for each commit, find the output in the wallet and the corresponding
	// api output (if it exists) and refresh it in-place in the wallet.
	// Note: minimizing the time we spend holding the wallet lock.
	{
		let last_confirmed_height = wallet.last_confirmed_height()?;
		// If the server height is less than our confirmed height, don't apply
		// these changes as the chain is syncing, incorrect or forking
		if height < last_confirmed_height {
			warn!(
				"Not updating outputs as the height of the node's chain \
				 is less than the last reported wallet update height."
			);
			warn!("Please wait for sync on node to complete or fork to resolve and try again.");
			return Ok(());
		}
		let mut batch = wallet.batch()?;
		for (commit, (id, mmr_index)) in wallet_outputs.iter() {
			if let Ok(mut output) = batch.get(id, mmr_index) {
				match api_outputs.get(&commit) {
					Some(o) => {
						// if this is a coinbase tx being confirmed, it's recordable in tx log
						if output.is_coinbase && output.status == OutputStatus::Unconfirmed {
							let log_id = batch.next_tx_log_id(parent_key_id)?;
							let mut t = TxLogEntry::new(
								parent_key_id.clone(),
								TxLogEntryType::ConfirmedCoinbase,
								log_id,
							);
							t.confirmed = true;
							t.amount_credited = output.value;
							t.amount_debited = 0;
							t.num_outputs = 1;
							t.update_confirmation_ts();
							output.tx_log_entry = Some(log_id);
							batch.save_tx_log_entry(t, &parent_key_id)?;
						}
						// also mark the transaction in which this output is involved as confirmed
						// note that one involved input/output confirmation SHOULD be enough
						// to reliably confirm the tx
						if !output.is_coinbase && output.status == OutputStatus::Unconfirmed {
							let tx = batch.tx_log_iter().find(|t| {
								Some(t.id) == output.tx_log_entry
									&& t.parent_key_id == *parent_key_id
							});
							if let Some(mut t) = tx {
								t.update_confirmation_ts();
								t.confirmed = true;
								batch.save_tx_log_entry(t, &parent_key_id)?;
							}
						}
						output.height = o.1;
						output.mark_unspent();
					}
					None => output.mark_spent(),
				};
				batch.save(output)?;
			}
		}
		{
			batch.save_last_confirmed_height(parent_key_id, height)?;
		}
		batch.commit()?;
	}
	Ok(())
}

/// Builds a single api query to retrieve the latest output data from the node.
/// So we can refresh the local wallet outputs.
fn refresh_output_state<T: ?Sized, C, K>(
	wallet: &mut T,
	height: u64,
	parent_key_id: &Identifier,
	update_all: bool,
) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	debug!("Refreshing wallet outputs");

	// build a local map of wallet outputs keyed by commit
	// and a list of outputs we want to query the node for
	let wallet_outputs = map_wallet_outputs(wallet, parent_key_id, update_all)?;

	let wallet_output_keys = wallet_outputs.keys().map(|commit| commit.clone()).collect();

	let api_outputs = wallet
		.w2n_client()
		.get_outputs_from_node(wallet_output_keys)?;
	apply_api_outputs(wallet, &wallet_outputs, &api_outputs, height, parent_key_id)?;
	clean_old_unconfirmed(wallet, height)?;
	Ok(())
}

fn clean_old_unconfirmed<T: ?Sized, C, K>(wallet: &mut T, height: u64) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	if height < 50 {
		return Ok(());
	}
	let mut ids_to_del = vec![];
	for out in wallet.iter() {
		if out.status == OutputStatus::Unconfirmed
			&& out.height > 0
			&& out.height < height - 50
			&& out.is_coinbase
		{
			ids_to_del.push(out.key_id.clone())
		}
	}
	let mut batch = wallet.batch()?;
	for id in ids_to_del {
		batch.delete(&id, &None)?;
	}
	batch.commit()?;
	Ok(())
}

/// Retrieve summary info about the wallet
/// caller should refresh first if desired
pub fn retrieve_info<T: ?Sized, C, K>(
	wallet: &mut T,
	parent_key_id: &Identifier,
	minimum_confirmations: u64,
) -> Result<WalletInfo, Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let current_height = wallet.last_confirmed_height()?;
	let outputs = wallet
		.iter()
		.filter(|out| out.root_key_id == *parent_key_id);

	let mut unspent_total = 0;
	let mut immature_total = 0;
	let mut unconfirmed_total = 0;
	let mut locked_total = 0;

	for out in outputs {
		match out.status {
			OutputStatus::Unspent => {
				if out.is_coinbase && out.lock_height > current_height {
					immature_total += out.value;
				} else if out.num_confirmations(current_height) < minimum_confirmations {
					// Treat anything less than minimum confirmations as "unconfirmed".
					unconfirmed_total += out.value;
				} else {
					unspent_total += out.value;
				}
			}
			OutputStatus::Unconfirmed => {
				// We ignore unconfirmed coinbase outputs completely.
				if !out.is_coinbase {
					if minimum_confirmations == 0 {
						unspent_total += out.value;
					} else {
						unconfirmed_total += out.value;
					}
				}
			}
			OutputStatus::Locked => {
				locked_total += out.value;
			}
			OutputStatus::Spent => {}
		}
	}

	Ok(WalletInfo {
		last_confirmed_height: current_height,
		minimum_confirmations,
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
	C: NodeClient,
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
	C: NodeClient,
	K: Keychain,
{
	let height = block_fees.height;
	let lock_height = height + global::coinbase_maturity();
	let key_id = block_fees.key_id();
	let parent_key_id = wallet.parent_key_id();

	let key_id = match key_id {
		Some(key_id) => keys::retrieve_existing_key(wallet, key_id, None)?.0,
		None => keys::next_available_key(wallet)?,
	};

	{
		// Now acquire the wallet lock and write the new output.
		let amount = reward(block_fees.fees);
		let commit = wallet.calc_commit_for_cache(amount, &key_id)?;
		let mut batch = wallet.batch()?;
		batch.save(OutputData {
			root_key_id: parent_key_id,
			key_id: key_id.clone(),
			n_child: key_id.to_path().last_path_index(),
			mmr_index: None,
			commit: commit,
			value: amount,
			status: OutputStatus::Unconfirmed,
			height: height,
			lock_height: lock_height,
			is_coinbase: true,
			tx_log_entry: None,
		})?;
		batch.commit()?;
	}

	debug!(
		"receive_coinbase: built candidate output - {:?}, {}",
		key_id.clone(),
		key_id,
	);

	let mut block_fees = block_fees.clone();
	block_fees.key_id = Some(key_id.clone());

	debug!("receive_coinbase: {:?}", block_fees);

	let (out, kern) = reward::output(wallet.keychain(), &key_id, block_fees.fees).unwrap();
	/* .context(ErrorKind::Keychain)?; */
	Ok((out, kern, block_fees))
}
