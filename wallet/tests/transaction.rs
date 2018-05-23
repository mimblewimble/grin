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
extern crate serde;
extern crate time;
extern crate uuid;

mod common;

use std::fs;
use std::sync::Arc;

use chain::Chain;
use chain::types::*;
use core::global::ChainTypes;
use core::{global, pow};
use util::LOGGER;
use wallet::grinwallet::selection;
use wallet::libwallet::aggsig;

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

/// Build and test new version of sending API
#[test]
fn build_transaction_2() {
	let chain = setup("test_output", "build_transaction_2/.grin");
	let wallet1 = common::create_wallet("test_output/build_transaction_2/wallet1");
	let wallet2 = common::create_wallet("test_output/build_transaction_2/wallet2");
	common::award_blocks_to_wallet(&chain, &wallet1, 10);
	// Wallet 1 has 600 Grins, wallet 2 has 0. Create a transaction that sends
	// 300 Grins from wallet 1 to wallet 2, using libwallet

	// Get lock height
	let chain_tip = chain.head().unwrap();
	let amount = 300_000_000_000;

	// ensure outputs we're selecting are up to date
	let res = common::refresh_output_state_local(&wallet1.0, &wallet1.1, &chain);

	if let Err(e) = res {
		panic!("Unable to refresh sender wallet outputs: {}", e);
	}

	// TRANSACTION WORKFLOW STARTS HERE
	// Sender creates a new aggsig context
	let mut sender_context_manager = aggsig::ContextManager::new();
	// Sender selects outputs into a new slate and save our corresponding IDs in
	// their transaction context. The secret key in our transaction context will be
	// randomly selected. This returns the public slate, and a closure that locks
	// our inputs and outputs once we're convinced the transaction exchange went
	// according to plan
	// This function is just a big helper to do all of that, in theory
	// this process can be split up in any way
	let (mut slate, sender_lock_fn) = selection::build_send_tx_slate(
		&wallet1.0,
		&wallet1.1,
		&mut sender_context_manager,
		2,
		amount,
		chain_tip.height,
		3,
		chain_tip.height,
		1000,
		true,
	).unwrap();

	// Generate a kernel offset and subtract from our context's secret key. Store
	// the offset in the slate's transaction kernel, and adds our public key
	// information to the slate
	let _ = slate
		.fill_round_1(&wallet1.1, &mut sender_context_manager, 0)
		.unwrap();

	debug!(LOGGER, "Transaction Slate after step 1: sender initiation");
	debug!(LOGGER, "-----------------------------------------");
	debug!(LOGGER, "{:?}", slate);

	// RECIPIENT (Handle sender initiation)
	let mut recipient_context_manager = aggsig::ContextManager::new();

	// Now, just like the sender did, recipient is going to select a target output,
	// add it to the transaction, and keep track of the corresponding wallet
	// Identifier Again, this is a helper to do that, which returns a closure that
	// creates the output when we're satisified the process was successful
	let (_, receiver_create_fn) = selection::build_recipient_output_with_slate(
		&wallet2.0,
		&wallet2.1,
		&mut recipient_context_manager,
		&mut slate,
	).unwrap();

	let _ = slate
		.fill_round_1(&wallet2.1, &mut recipient_context_manager, 1)
		.unwrap();

	// recipient can proceed to round 2 now
	let _ = receiver_create_fn();

	let _ = slate
		.fill_round_2(&wallet2.1, &mut recipient_context_manager, 1)
		.unwrap();

	debug!(
		LOGGER,
		"Transaction Slate after step 2: receiver initiation"
	);
	debug!(LOGGER, "-----------------------------------------");
	debug!(LOGGER, "{:?}", slate);

	// SENDER Part 3: Sender confirmation
	let _ = slate
		.fill_round_2(&wallet1.1, &mut sender_context_manager, 0)
		.unwrap();

	debug!(LOGGER, "PartialTx after step 3: sender confirmation");
	debug!(LOGGER, "--------------------------------------------");
	debug!(LOGGER, "{:?}", slate);

	// Final transaction can be built by anyone at this stage
	let res = slate.finalize(&wallet1.1);

	if let Err(e) = res {
		panic!("Error creating final tx: {:?}", e);
	}

	debug!(LOGGER, "Final transaction is:");
	debug!(LOGGER, "--------------------------------------------");
	debug!(LOGGER, "{:?}", slate.tx);

	// All okay, lock sender's outputs
	let _ = sender_lock_fn();

	// Insert this transaction into a new block, then mine till confirmation
	common::award_block_to_wallet(&chain, vec![&slate.tx], &wallet1);
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
