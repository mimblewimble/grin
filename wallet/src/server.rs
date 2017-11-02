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


use api::ApiServer;
use keychain::Keychain;
use handlers::CoinbaseHandler;
use receiver::WalletReceiver;
use types::WalletConfig;
use util::LOGGER;

pub fn start_rest_apis(wallet_config: WalletConfig, keychain: Keychain) {
	info!(
		LOGGER,
		"Starting the Grin wallet receiving daemon at {}...",
		wallet_config.api_listen_addr()
	);

	let receive_tx_handler = WalletReceiver {
		config: wallet_config.clone(),
		keychain: keychain.clone(),
	};
	let coinbase_handler = CoinbaseHandler {
		config: wallet_config.clone(),
		keychain: keychain.clone(),
	};

	let router = router!(
		receive_tx: post "/receive/transaction" => receive_tx_handler,
		receive_coinbase: post "/receive/coinbase" => coinbase_handler,
	);

	let mut apis = ApiServer::new("/v1".to_string());
	apis.register_handler(router);
	apis.start(wallet_config.api_listen_addr()).unwrap_or_else(|e| {
		error!(LOGGER, "Failed to start Grin wallet receiver: {}.", e);
	});
}
