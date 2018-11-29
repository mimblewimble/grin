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

use util;
use uuid::Uuid;

use core::ser;
use keychain::{Identifier, Keychain};
use libtx::slate::Slate;
use libwallet::internal::{selection, updater};
use libwallet::types::{Context, NodeClient, TxLogEntryType, WalletBackend};
use libwallet::{Error, ErrorKind};

/// Receive a transaction, modifying the slate accordingly (which can then be
/// sent back to sender for posting)
pub fn receive_tx<T: ?Sized, C, K>(
	wallet: &mut T,
	slate: &mut Slate,
	parent_key_id: &Identifier,
	is_self: bool,
) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	// create an output using the amount in the slate
	let (_, mut context, receiver_create_fn) = selection::build_recipient_output_with_slate(
		wallet,
		slate,
		parent_key_id.clone(),
		is_self,
	)?;

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
pub fn create_send_tx<T: ?Sized, C, K>(
	wallet: &mut T,
	amount: u64,
	minimum_confirmations: u64,
	max_outputs: usize,
	num_change_outputs: usize,
	selection_strategy_is_use_all: bool,
	parent_key_id: &Identifier,
	is_self: bool,
) -> Result<
	(
		Slate,
		Context,
		impl FnOnce(&mut T, &str) -> Result<(), Error>,
	),
	Error,
>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	// Get lock height
	let current_height = wallet.w2n_client().get_chain_height()?;
	// ensure outputs we're selecting are up to date
	updater::refresh_outputs(wallet, parent_key_id)?;

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
		num_change_outputs,
		selection_strategy_is_use_all,
		parent_key_id.clone(),
		is_self,
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
pub fn complete_tx<T: ?Sized, C, K>(
	wallet: &mut T,
	slate: &mut Slate,
	context: &Context,
) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
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

/// Rollback outputs associated with a transaction in the wallet
pub fn cancel_tx<T: ?Sized, C, K>(
	wallet: &mut T,
	parent_key_id: &Identifier,
	tx_id: Option<u32>,
	tx_slate_id: Option<Uuid>,
) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let mut tx_id_string = String::new();
	if let Some(tx_id) = tx_id {
		tx_id_string = tx_id.to_string();
	} else if let Some(tx_slate_id) = tx_slate_id {
		tx_id_string = tx_slate_id.to_string();
	}
	let tx_vec = updater::retrieve_txs(wallet, tx_id, tx_slate_id, &parent_key_id)?;
	if tx_vec.len() != 1 {
		return Err(ErrorKind::TransactionDoesntExist(tx_id_string))?;
	}
	let tx = tx_vec[0].clone();
	if tx.tx_type != TxLogEntryType::TxSent && tx.tx_type != TxLogEntryType::TxReceived {
		return Err(ErrorKind::TransactionNotCancellable(tx_id_string))?;
	}
	if tx.confirmed == true {
		return Err(ErrorKind::TransactionNotCancellable(tx_id_string))?;
	}
	// get outputs associated with tx
	let res = updater::retrieve_outputs(wallet, false, Some(tx.id), &parent_key_id)?;
	let outputs = res.iter().map(|(out, _)| out).cloned().collect();
	updater::cancel_tx_and_outputs(wallet, tx, outputs, parent_key_id)?;
	Ok(())
}

/// Retrieve the associated stored finalised hex Transaction for a given transaction Id
/// as well as whether it's been confirmed
pub fn retrieve_tx_hex<T: ?Sized, C, K>(
	wallet: &mut T,
	parent_key_id: &Identifier,
	tx_id: u32,
) -> Result<(bool, Option<String>), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let tx_vec = updater::retrieve_txs(wallet, Some(tx_id), None, parent_key_id)?;
	if tx_vec.len() != 1 {
		return Err(ErrorKind::TransactionDoesntExist(tx_id.to_string()))?;
	}
	let tx = tx_vec[0].clone();
	Ok((tx.confirmed, tx.tx_hex))
}

/// Update the stored hex transaction (this update needs to happen when the TX is finalised)
pub fn update_tx_hex<T: ?Sized, C, K>(
	wallet: &mut T,
	parent_key_id: &Identifier,
	slate: &Slate,
) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let tx_hex = util::to_hex(ser::ser_vec(&slate.tx).unwrap());
	let tx_vec = updater::retrieve_txs(wallet, None, Some(slate.id), parent_key_id)?;
	if tx_vec.len() != 1 {
		return Err(ErrorKind::TransactionDoesntExist(slate.id.to_string()))?;
	}
	let mut tx = tx_vec[0].clone();
	tx.tx_hex = Some(tx_hex);
	let batch = wallet.batch()?;
	batch.save_tx_log_entry(tx, &parent_key_id)?;
	batch.commit()?;
	Ok(())
}

#[cfg(test)]
mod test {
	use keychain::{ExtKeychain, ExtKeychainPath, Keychain};
	use libtx::build;

	#[test]
	// demonstrate that input.commitment == referenced output.commitment
	// based on the public key and amount begin spent
	fn output_commitment_equals_input_commitment_on_spend() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();

		let tx1 = build::transaction(vec![build::output(105, key_id1.clone())], &keychain).unwrap();
		let tx2 = build::transaction(vec![build::input(105, key_id1.clone())], &keychain).unwrap();

		assert_eq!(tx1.outputs()[0].features, tx2.inputs()[0].features);
		assert_eq!(tx1.outputs()[0].commitment(), tx2.inputs()[0].commitment());
	}
}
