// Copyright 2016 The Grin Developers
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

use std::convert::From;
use secp::{self, Secp256k1};
use secp::key::SecretKey;

use checker;
use core::core::{Transaction, build};
use extkey::ExtendedKey;
use types::*;

pub fn issue_send_tx(ext_key: &ExtendedKey, amount: u64, dest: String) -> Result<(), Error> {
  checker::refresh_outputs(&WalletConfig::default(), ext_key);

	let (tx, blind_sum) = build_send_tx(ext_key, amount)?;
	let json_tx = partial_tx_to_json(amount, blind_sum, tx);
	if dest == "stdout" {
		println!("{}", dest);
	} else if &dest[..4] == "http" {
		// TODO
		unimplemented!();
	}
	Ok(())
}

/// Builds a transaction to send to someone from the HD seed associated with the
/// wallet and the amount to send. Handles reading through the wallet data file,
/// selecting outputs to spend and building the change.
fn build_send_tx(ext_key: &ExtendedKey, amount: u64) -> Result<(Transaction, SecretKey), Error> {
	// first, rebuild the private key from the seed
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);

	// second, check from our local wallet data for outputs to spend
	let mut wallet_data = WalletData::read()?;
	let (mut coins, change) = wallet_data.select(ext_key.fingerprint, amount);
	if change < 0 {
		return Err(Error::NotEnoughFunds((-change) as u64));
	}

	// TODO add fees, which is likely going to make this iterative

	// third, build inputs using the appropriate key
	let mut parts = vec![];
	for coin in &coins {
		let in_key = ext_key.derive(&secp, coin.n_child).map_err(|e| Error::Key(e))?;
		parts.push(build::input(coin.value, in_key.key));
	}

	// fourth, derive a new private for change and build the change output
	let next_child = wallet_data.next_child(ext_key.fingerprint);
	let change_key = ext_key.derive(&secp, next_child).map_err(|e| Error::Key(e))?;
	parts.push(build::output(change as u64, change_key.key));

	// we got that far, time to start tracking the new output, finalize tx
	// and lock the outputs used
	wallet_data.append_output(OutputData {
		fingerprint: change_key.fingerprint,
		n_child: change_key.n_child,
		value: change as u64,
		status: OutputStatus::Unconfirmed,
	});
	for mut coin in coins {
		coin.lock();
	}
	wallet_data.write()?;
	build::transaction(parts).map_err(&From::from)
}
