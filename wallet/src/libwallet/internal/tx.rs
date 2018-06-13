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

//! Transaction building functions

use core::core::Transaction;
use keychain::{Identifier, Keychain};
use libtx::slate::Slate;
use libtx::{build, tx_fee};
use libwallet::internal::{selection, sigcontext, updater};
use libwallet::types::{WalletBackend, WalletClient};
use libwallet::{Error, ErrorKind};
use util::LOGGER;

/// Receive a transaction, modifying the slate accordingly (which can then be
/// sent back to sender for posting)
pub fn receive_tx<T, K>(wallet: &mut T, slate: &mut Slate) -> Result<(), Error>
where
	T: WalletBackend<K>,
	K: Keychain,
{
	// create an output using the amount in the slate
	let (_, mut context, receiver_create_fn) =
		selection::build_recipient_output_with_slate(wallet, slate).unwrap();

	// fill public keys
	let _ = slate.fill_round_1(
		wallet.keychain(),
		&mut context.sec_key,
		&context.sec_nonce,
		1,
	)?;

	// perform partial sig
	let _ = slate.fill_round_2(wallet.keychain(), &context.sec_key, &context.sec_nonce, 1)?;

	// Save output in wallet
	let _ = receiver_create_fn(wallet);

	Ok(())
}

/// Issue a new transaction to the provided sender by spending some of our
/// wallet
pub fn create_send_tx<T, K>(
	wallet: &mut T,
	amount: u64,
	minimum_confirmations: u64,
	max_outputs: usize,
	selection_strategy_is_use_all: bool,
) -> Result<
	(
		Slate,
		sigcontext::Context,
		impl FnOnce(&mut T) -> Result<(), Error>,
	),
	Error,
>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	// Get lock height
	let current_height = wallet.get_chain_height(wallet.node_url())?;
	// ensure outputs we're selecting are up to date
	updater::refresh_outputs(wallet)?;

	let lock_height = current_height;

	// Sender selects outputs into a new slate and save our corresponding keys in
	// a transaction context. The secret key in our transaction context will be
	// randomly selected. This returns the public slate, and a closure that locks
	// our inputs and outputs once we're convinced the transaction exchange went
	// according to plan
	// This function is just a big helper to do all of that, in theory
	// this process can be split up in any way
	let (mut slate, mut context, sender_lock_fn) = selection::build_send_tx_slate(
		wallet,
		2,
		amount,
		current_height,
		minimum_confirmations,
		lock_height,
		max_outputs,
		selection_strategy_is_use_all,
	)?;

	// Generate a kernel offset and subtract from our context's secret key. Store
	// the offset in the slate's transaction kernel, and adds our public key
	// information to the slate
	let _ = slate.fill_round_1(
		wallet.keychain(),
		&mut context.sec_key,
		&context.sec_nonce,
		0,
	)?;

	Ok((slate, context, sender_lock_fn))
}

/// Complete a transaction as the sender
pub fn complete_tx<T, K>(
	wallet: &mut T,
	slate: &mut Slate,
	context: &sigcontext::Context,
) -> Result<(), Error>
where
	T: WalletBackend<K>,
	K: Keychain,
{
	let _ = slate.fill_round_2(wallet.keychain(), &context.sec_key, &context.sec_nonce, 0)?;
	// Final transaction can be built by anyone at this stage
	let res = slate.finalize(wallet.keychain());
	if let Err(e) = res {
		Err(ErrorKind::LibTX(e.kind()))?
	}
	Ok(())
}

/// Issue a burn tx
pub fn issue_burn_tx<T, K>(
	wallet: &mut T,
	amount: u64,
	minimum_confirmations: u64,
	max_outputs: usize,
) -> Result<Transaction, Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	// TODO
	// let keychain = &Keychain::burn_enabled(wallet.keychain(),
	// &Identifier::zero());
	let keychain = wallet.keychain().clone();

	let current_height = wallet.get_chain_height(wallet.node_url())?;

	let _ = updater::refresh_outputs(wallet);

	let key_id = keychain.root_key_id();

	// select some spendable coins from the wallet
	let coins = wallet.read_wallet(|wallet_data| {
		Ok(wallet_data.select_coins(
			key_id.clone(),
			amount,
			current_height,
			minimum_confirmations,
			max_outputs,
			false,
		))
	})?;

	debug!(LOGGER, "selected some coins - {}", coins.len());

	let fee = tx_fee(coins.len(), 2, selection::coins_proof_count(&coins), None);
	let (mut parts, _) = selection::inputs_and_change(&coins, wallet, current_height, amount, fee)?;

	// add burn output and fees
	parts.push(build::output(amount - fee, Identifier::zero()));

	// finalize the burn transaction and send
	let tx_burn = build::transaction(parts, &keychain)?;
	tx_burn.validate()?;
	Ok(tx_burn)
}

#[cfg(test)]
mod test {
	use keychain::{ExtKeychain, Keychain};
	use libtx::build;

	#[test]
	// demonstrate that input.commitment == referenced output.commitment
	// based on the public key and amount begin spent
	fn output_commitment_equals_input_commitment_on_spend() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();

		let tx1 = build::transaction(vec![build::output(105, key_id1.clone())], &keychain).unwrap();
		let tx2 = build::transaction(vec![build::input(105, key_id1.clone())], &keychain).unwrap();

		assert_eq!(tx1.outputs[0].features, tx2.inputs[0].features);
		assert_eq!(tx1.outputs[0].commitment(), tx2.inputs[0].commitment());
	}
}
