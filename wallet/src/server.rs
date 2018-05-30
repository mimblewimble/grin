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

use api::ApiServer;
use handlers::CoinbaseHandler;
use iron::Handler;
use libwallet::types::WalletBackend;
use receiver::WalletReceiver;
use std::sync::{Arc, RwLock};
use util::LOGGER;

pub fn start_rest_apis<T>(in_wallet: T, api_listen_addr: &str)
where
	T: WalletBackend,
	CoinbaseHandler<T>: Handler,
	WalletReceiver<T>: Handler,
{
	info!(
		LOGGER,
		"Starting the Grin wallet receiving daemon at {}...", api_listen_addr
	);

	let wallet = Arc::new(RwLock::new(in_wallet));

	let receive_tx_handler = WalletReceiver {
		wallet: wallet.clone(),
	};
	let coinbase_handler = CoinbaseHandler {
		wallet: wallet.clone(),
	};

	let router = router!(
		receive_tx: post "/receive/transaction" => receive_tx_handler,
		receive_coinbase: post "/receive/coinbase" => coinbase_handler,
	);

	let mut apis = ApiServer::new("/v1".to_string());
	apis.register_handler(router);
	match apis.start(api_listen_addr) {
		Err(e) => error!(LOGGER, "Failed to start Grin wallet listener: {}.", e),
		Ok(_) => info!(LOGGER, "Wallet listener started"),
	};
}
