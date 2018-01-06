// Copyright 2017 The Grin Developers
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

use iron::prelude::*;
use iron::Handler;
use iron::status;
use serde_json;
use bodyparser;

use receiver::receive_coinbase;
use core::ser;
use api;
use keychain::Keychain;
use types::*;
use util;

use checker;
use receiver::receive_json_tx;
use core::core::amount_to_hr_string;
use sender::send_json_tx_str;

pub struct CoinbaseHandler {
	pub config: WalletConfig,
	pub keychain: Keychain,
}

impl CoinbaseHandler {
	fn build_coinbase(&self, block_fees: &BlockFees) -> Result<CbData, Error> {
		let (out, kern, block_fees) = receive_coinbase(&self.config, &self.keychain, block_fees)
			.map_err(|e| {
				api::Error::Internal(format!("Error building coinbase: {:?}", e))
			})?;

		let out_bin = ser::ser_vec(&out).map_err(|e| {
			api::Error::Internal(format!("Error serializing output: {:?}", e))
		})?;

		let kern_bin = ser::ser_vec(&kern).map_err(|e| {
			api::Error::Internal(format!("Error serializing kernel: {:?}", e))
		})?;

		let key_id_bin = match block_fees.key_id {
			Some(key_id) => ser::ser_vec(&key_id).map_err(|e| {
				api::Error::Internal(format!("Error serializing kernel: {:?}", e))
			})?,
			None => vec![],
		};

		Ok(CbData {
			output: util::to_hex(out_bin),
			kernel: util::to_hex(kern_bin),
			key_id: util::to_hex(key_id_bin),
		})
	}
}

// TODO - error handling - what to return if we fail to get the wallet lock for
// some reason...
impl Handler for CoinbaseHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let struct_body = req.get::<bodyparser::Struct<BlockFees>>();

		if let Ok(Some(block_fees)) = struct_body {
			let coinbase = self.build_coinbase(&block_fees)
				.map_err(|e| IronError::new(e, status::BadRequest))?;
			if let Ok(json) = serde_json::to_string(&coinbase) {
				Ok(Response::with((status::Ok, json)))
			} else {
				Ok(Response::with((status::BadRequest, "")))
			}
		} else {
			Ok(Response::with((status::BadRequest, "")))
		}
	}
}

/// Component used to receive coins, implements all the receiving end of the
/// wallet REST API as well as some of the command-line operations.
#[derive(Clone)]
pub struct WalletReceiverHandler {
    pub keychain: Keychain,
    pub config: WalletConfig,
}

impl Handler for WalletReceiverHandler {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let struct_body = req.get::<bodyparser::Struct<PartialTx>>();

        if let Ok(Some(partial_tx)) = struct_body {
            receive_json_tx(&self.config, &self.keychain, &partial_tx)
                .map_err(|e| {
                    api::Error::Internal(
                        format!("Error processing partial transaction: {:?}", e),
                    )})
                .unwrap();
            Ok(Response::with(status::Ok))
        } else {
            Ok(Response::with((status::BadRequest, "")))
        }
    }
}

pub struct InfoHandler {
    pub config: WalletConfig,
}

impl InfoHandler {
    fn get_info(&self, info_request: &InfoRequest) -> Result<WalletInfoData, Error> {

        let wallet_seed =
            WalletSeed::from_file(&self.config).expect("Failed to read wallet seed file.");

        let keychain = wallet_seed.derive_keychain(&info_request.passphrase).expect(
            "Failed to derive keychain from seed file and passphrase.",
        );

        let mut unspent_total=0;
        let mut unspent_but_locked_total=0;
        let mut unconfirmed_total=0;
        let mut locked_total=0;

        let _ = WalletData::read_wallet(&self.config.data_file_dir, |wallet_data| {
            let current_height = match checker::get_tip_from_node(&self.config) {
                Ok(tip) => tip.height,
                Err(_) => match wallet_data.outputs.values().map(|out| out.height).max() {
                    Some(height) => height,
                    None => 0,
                },
            };

            for out in wallet_data
                .outputs
                .values()
                .filter(|out| out.root_key_id == keychain.root_key_id())
                {
                    if out.status == OutputStatus::Unspent {
                        unspent_total+=out.value;
                        if out.lock_height > current_height {
                            unspent_but_locked_total+=out.value;
                        }
                    }
                    if out.status == OutputStatus::Unconfirmed && !out.is_coinbase {
                        unconfirmed_total+=out.value;
                    }
                    if out.status == OutputStatus::Locked {
                        locked_total+=out.value;
                    }
                };

        });

        Ok(WalletInfoData {
            unspent_total: amount_to_hr_string(unspent_total),
            unspent_but_locked_total: amount_to_hr_string(unspent_but_locked_total),
            unconfirmed_total: amount_to_hr_string(unconfirmed_total),
            locked_total: amount_to_hr_string(locked_total),
        })
    }
}

impl Handler for InfoHandler {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let struct_body = req.get::<bodyparser::Struct<InfoRequest>>();

        if let Ok(Some(info_request)) = struct_body {
            let info = self.get_info(&info_request)
                .map_err(|e| IronError::new(e, status::BadRequest))?;
            if let Ok(json) = serde_json::to_string(&info) {
                Ok(Response::with((status::Ok, json)))
            } else {
                Ok(Response::with((status::BadRequest, "")))
            }
        }
        else
        {
            Ok(Response::with((status::BadRequest, "")))
        }
    }
}

pub struct WalletSenderHandler {
    pub config: WalletConfig,
}

impl WalletSenderHandler {
    fn send_tx(&self, send_tx: &SendTx) -> Result<WalletSendResult, Error> {

        let wallet_seed =
            WalletSeed::from_file(&self.config).expect("Failed to read wallet seed file.");

        let keychain = wallet_seed.derive_keychain(&send_tx.passphrase).expect(
            "Failed to derive keychain from seed file and passphrase.",
        );

        send_json_tx_str(&self.config, &keychain, &send_tx)
        .map_err(|e| {
        api::Error::Internal(
            format!("Error sending transaction : {:?}", e),
        )})
        .unwrap();

        Ok(WalletSendResult {
            confirmed: true,
        })

    }
}

impl Handler for WalletSenderHandler {
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        let struct_body = req.get::<bodyparser::Struct<SendTx>>();

        if let Ok(Some(send_tx)) = struct_body {
            let result = self.send_tx(&send_tx)
                .map_err(|e| { IronError::new(e, status::BadRequest) })?;
            if let Ok(json) = serde_json::to_string(&result) {
                Ok(Response::with((status::Ok, json)))
            } else {
                Ok(Response::with((status::BadRequest, "")))
            }
        } else {
            Ok(Response::with((status::BadRequest, "")))
        }
    }
}