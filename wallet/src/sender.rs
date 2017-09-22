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
use secp::{self};
use secp::key::SecretKey;

use checker;
use core::core::{Transaction, build};
use extkey::ExtendedKey;
use types::*;

use api;

/// Issue a new transaction to the provided sender by spending some of our
/// wallet
/// UTXOs. The destination can be "stdout" (for command line) or a URL to the
/// recipients wallet receiver (to be implemented).
pub fn issue_send_tx(config: &WalletConfig, ext_key: &ExtendedKey, amount: u64, dest: String) -> Result<(), Error> {
	let _ = checker::refresh_outputs(&config, ext_key);

	let (tx, blind_sum) = build_send_tx(config, ext_key, amount)?;
	let json_tx = partial_tx_to_json(amount, blind_sum, tx);

	if dest == "stdout" {
		println!("{}", json_tx);
	} else if &dest[..4] == "http" {
		let url = format!("{}/v1/receive/receive_json_tx", &dest);
		debug!("Posting partial transaction to {}", url);
		let request = WalletReceiveRequest::PartialTransaction(json_tx);
		let _: CbData = api::client::post(url.as_str(), &request)
			.expect(&format!("Wallet receiver at {} unreachable, could not send transaction. Is it running?", url));
	}
	Ok(())
}

/// Builds a transaction to send to someone from the HD seed associated with the
/// wallet and the amount to send. Handles reading through the wallet data file,
/// selecting outputs to spend and building the change.
fn build_send_tx(config: &WalletConfig, ext_key: &ExtendedKey, amount: u64) -> Result<(Transaction, SecretKey), Error> {
	// first, rebuild the private key from the seed
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);

	// operate within a lock on wallet data
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {

		// second, check from our local wallet data for outputs to spend
		let (coins, change) = wallet_data.select(&ext_key.fingerprint, amount);
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
		let next_child = wallet_data.next_child(&ext_key.fingerprint);
		let change_key = ext_key.derive(&secp, next_child).map_err(|e| Error::Key(e))?;
		parts.push(build::output(change as u64, change_key.key));

		// we got that far, time to start tracking the new output, finalize tx
		// and lock the outputs used
		wallet_data.append_output(OutputData {
			fingerprint: change_key.fingerprint,
			n_child: change_key.n_child,
			value: change as u64,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
		});
		for mut coin in coins {
			coin.lock();
		}

		build::transaction(parts).map_err(&From::from)
	})?
}

#[cfg(test)]
mod test {
	extern crate rustc_serialize as serialize;

	use core::core::build::{input, output, transaction};
	use types::{OutputData, OutputStatus};

	use secp::Secp256k1;
	use super::ExtendedKey;
	use self::serialize::hex::FromHex;

	#[test]
	// demonstrate that input.commitment == referenced output.commitment
	// based on the wallet extended key and the coin being spent
	fn output_commitment_equals_input_commitment_on_spend() {
		let secp = Secp256k1::new();
		let seed = "000102030405060708090a0b0c0d0e0f".from_hex().unwrap();

		let ext_key = ExtendedKey::from_seed(&secp, &seed.as_slice()).unwrap();

		let out_key = ext_key.derive(&secp, 1).unwrap();

		let coin = OutputData {
			fingerprint: out_key.fingerprint,
			n_child: out_key.n_child,
			value: 5,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
		};

		let (tx, _) = transaction(vec![output(coin.value, out_key.key)]).unwrap();

		let in_key = ext_key.derive(&secp, coin.n_child).unwrap();

		let (tx2, _) = transaction(vec![input(coin.value, in_key.key)]).unwrap();

		assert_eq!(in_key.key, out_key.key);
		assert_eq!(tx.outputs[0].commitment(), tx2.inputs[0].commitment());
	}
}
