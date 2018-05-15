// Copyright 2018 The Grin Developers
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

//! tests for transactions building within libwallet
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
extern crate rand;
#[macro_use]
extern crate slog;
extern crate time;
extern crate uuid;

mod common;

use std::fs;
use std::sync::Arc;

use uuid::Uuid;

use chain::Chain;
use chain::types::*;
use core::{global, pow};
use core::global::ChainTypes;
use wallet::libwallet::{aggsig, transaction};
use wallet::grinwallet::{keys, selection};
use wallet::types::{OutputData, OutputStatus, WalletData};
use util::LOGGER;

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str, chain_dir: &str) -> Chain {
	util::init_test_logger();
	clean_output_dir(test_dir);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let genesis_block = pow::mine_genesis_block().unwrap();
	let dir_name = format!("{}/{}", test_dir, chain_dir);
	chain::Chain::init(
		dir_name.to_string(),
		Arc::new(NoopAdapter {}),
		genesis_block,
		pow::verify_size,
	).unwrap()
}

/// Build a transaction between 2 parties
#[cfg(test)]
#[test]
fn build_transaction() {
	let chain = setup("test_output", "build_transaction/.grin");
	let wallet1 = common::create_wallet("test_output/build_transaction/wallet1");
	let wallet2 = common::create_wallet("test_output/build_transaction/wallet2");
	common::award_blocks_to_wallet(&chain, &wallet1, 10);
	// Wallet 1 has 600 Grins, wallet 2 has 0. Create a transaction that sends
	// 300 Grins from wallet 1 to wallet 2, using libwallet
	// Sender creates a new aggsig context
	// SENDER (create sender initiation)
	let mut sender_context_manager = aggsig::ContextManager::new();
	let tx_id = Uuid::new_v4();

	// Get lock height
	let chain_tip = chain.head().unwrap();

	// ensure outputs we're selecting are up to date
	let res = common::refresh_output_state_local(&wallet1.0, &wallet1.1, &chain);
	let amount = 300_000_000_000;

	// Select our outputs
	let tx_data = selection::build_send_tx(
		&wallet1.0,
		&wallet1.1,
		amount,
		chain_tip.height,
		3,
		chain_tip.height,
		1000,
		true,
	).unwrap();

	if let Err(e) = res {
		panic!("Unable to refresh sender wallet outputs: {}", e);
	}

	let partial_tx = transaction::sender_initiation(
		&wallet1.1,
		&tx_id,
		&mut sender_context_manager,
		chain_tip.height,
		tx_data,
	).unwrap();

	let sender_context = sender_context_manager.get_context(&tx_id);

	// TODO: Might make more sense to do this before the transaction
	// building call
	// Closure to acquire wallet lock and lock the coins being spent
	// so we avoid accidental double spend attempt.
	let update_sender_wallet = || {
		WalletData::with_wallet(&wallet1.0.data_file_dir, |wallet_data| {
			for id in sender_context.get_outputs().clone() {
				let coin = wallet_data.get_output(&id).unwrap().clone();
				wallet_data.lock_output(&coin);
			}
		})
	};
	debug!(LOGGER, "PartialTx after step 1: sender initiation");
	debug!(LOGGER, "-----------------------------------------");
	debug!(LOGGER, "{:?}", partial_tx);

	// RECIPIENT (Handle sender initiation)
	let mut recipient_context_manager = aggsig::ContextManager::new();

	// Create a potential output for this transaction
	let (key_id, derivation) = WalletData::with_wallet(&wallet2.0.data_file_dir, |wallet_data| {
		keys::next_available_key(&wallet_data, &wallet2.1)
	}).unwrap();

	let partial_tx = transaction::recipient_initiation(
		&wallet2.1,
		&mut recipient_context_manager,
		&partial_tx,
		&key_id,
	).unwrap();
	let mut context = recipient_context_manager.get_context(&partial_tx.id);

	// Add the output to recipient's wallet
	let _ = WalletData::with_wallet(&wallet2.0.data_file_dir, |wallet_data| {
		wallet_data.add_output(OutputData {
			root_key_id: wallet2.1.root_key_id(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: partial_tx.amount - context.fee,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
			is_coinbase: false,
			block: None,
			merkle_proof: None,
		});
	}).unwrap();
	context.add_output(&key_id);
	recipient_context_manager.save_context(context);

	debug!(LOGGER, "PartialTx after step 2: recipient initiation");
	debug!(LOGGER, "--------------------------------------------");
	debug!(LOGGER, "{:?}", partial_tx);

	// TODO: We want to allow the sender to be able to calculate this, but also need
	// the recipient's output information available, and the recipient needs to know
	// whether to finalize the output in their wallet
	let _tx_with_recipients_pubkeys = partial_tx.clone();

	// SENDER Part 3: Sender confirmation
	let partial_tx =
		transaction::sender_confirmation(&wallet1.1, &mut sender_context_manager, partial_tx)
			.unwrap();

	debug!(LOGGER, "PartialTx after step 3: sender confirmation");
	debug!(LOGGER, "--------------------------------------------");
	debug!(LOGGER, "{:?}", partial_tx);

	// RECIPIENT Part 4: Recipient confirmation
	// Get output we created in earlier step
	let context = recipient_context_manager.get_context(&partial_tx.id);
	let output_vec = context.get_outputs();
	let root_key_id = &wallet2.1.root_key_id();

	// operate within a lock on wallet data
	let (key_id, derivation) = WalletData::with_wallet(&wallet2.0.data_file_dir, |wallet_data| {
		let (key_id, derivation) = keys::retrieve_existing_key(&wallet_data, output_vec[0].clone());

		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: partial_tx.amount - context.fee,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
			is_coinbase: false,
			block: None,
			merkle_proof: None,
		});

		(key_id, derivation)
	}).unwrap();

	let final_tx_recipient = transaction::finalize_transaction(
		&wallet2.1,
		&mut recipient_context_manager,
		&partial_tx,
		&partial_tx,
		&key_id,
		derivation,
	);

	if let Err(e) = final_tx_recipient {
		panic!("Error creating final tx: {:?}", e);
	}

	debug!(LOGGER, "Recipient calculates final transaction as:");
	debug!(LOGGER, "--------------------------------------------");
	debug!(LOGGER, "{:?}", final_tx_recipient);

	let _ = update_sender_wallet();

	// Insert this transaction into a new block, then mine till confirmation
	common::award_block_to_wallet(&chain, vec![&final_tx_recipient.unwrap()], &wallet1);
	common::award_blocks_to_wallet(&chain, &wallet1, 3);

	// Refresh wallets
	let res = common::refresh_output_state_local(&wallet2.0, &wallet2.1, &chain);
	if let Err(e) = res {
		panic!("Error refreshing output state for wallet: {:?}", e);
	}

	let chain_tip = chain.head().unwrap();
	let balances = common::get_wallet_balances(&wallet2.0, &wallet2.1, chain_tip.height).unwrap();

	assert_eq!(balances.3, 300_000_000_000);
}
