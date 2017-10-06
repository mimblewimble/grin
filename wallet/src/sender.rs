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

use api;
use checker;
use core::core::{Transaction, build};
use keychain::{BlindingFactor, Keychain};
use types::*;

/// Issue a new transaction to the provided sender by spending some of our
/// wallet
/// UTXOs. The destination can be "stdout" (for command line) or a URL to the
/// recipients wallet receiver (to be implemented).
pub fn issue_send_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	dest: String,
) -> Result<(), Error> {
	let _ = checker::refresh_outputs(config, keychain);

	let (tx, blind_sum) = build_send_tx(config, keychain, amount)?;
	let json_tx = partial_tx_to_json(amount, blind_sum, tx);

	if dest == "stdout" {
		println!("{}", json_tx);
	} else if &dest[..4] == "http" {
		let url = format!("{}/v1/receive/receive_json_tx", &dest);
		debug!("Posting partial transaction to {}", url);
		let request = WalletReceiveRequest::PartialTransaction(json_tx);
		let _: CbData = api::client::post(url.as_str(), &request)
			.expect(&format!("Wallet receiver at {} unreachable, could not send transaction. Is it running?", url));
	} else {
		panic!("dest not in expected format: {}", dest);
	}
	Ok(())
}

/// Builds a transaction to send to someone from the HD seed associated with the
/// wallet and the amount to send. Handles reading through the wallet data file,
/// selecting outputs to spend and building the change.
fn build_send_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
) -> Result<(Transaction, BlindingFactor), Error> {
	let fingerprint = keychain.clone().fingerprint();

	// operate within a lock on wallet data
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {

		// select some suitable outputs to spend from our local wallet
		let (coins, change) = wallet_data.select(fingerprint.clone(), amount);
		if change < 0 {
			return Err(Error::NotEnoughFunds((-change) as u64));
		}

		// TODO add fees, which is likely going to make this iterative

		// build inputs using the appropriate derived pubkeys
		let mut parts = vec![];
		for coin in &coins {
			let pubkey = keychain.derive_pubkey(coin.n_child)?;
			parts.push(build::input(coin.value, pubkey));
		}

		// derive an additional pubkey for change and build the change output
		let change_derivation = wallet_data.next_child(fingerprint.clone());
		let change_key = keychain.derive_pubkey(change_derivation)?;
		parts.push(build::output(change as u64, change_key.clone()));

		// we got that far, time to start tracking the new output, finalize tx
		// and lock the outputs used
		wallet_data.add_output(OutputData {
			fingerprint: fingerprint.clone(),
			identifier: change_key.clone(),
			n_child: change_derivation,
			value: change as u64,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
		});

		for coin in &coins {
			wallet_data.lock_output(coin);
		}

		let result = build::transaction(parts, &keychain)?;
		Ok(result)
	})?
}

#[cfg(test)]
mod test {
	use core::core::build::{input, output, transaction};
	use keychain::Keychain;

	#[test]
	// demonstrate that input.commitment == referenced output.commitment
	// based on the public key and amount begin spent
	fn output_commitment_equals_input_commitment_on_spend() {
		let keychain = Keychain::from_random_seed().unwrap();
		let pk1 = keychain.derive_pubkey(1).unwrap();

		let (tx, _) = transaction(
			vec![output(105, pk1.clone())],
			&keychain,
		).unwrap();

		let (tx2, _) = transaction(
			vec![input(105, pk1.clone())],
			&keychain,
		).unwrap();

		assert_eq!(tx.outputs[0].commitment(), tx2.inputs[0].commitment());
	}
}
