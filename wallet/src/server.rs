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

use hyper::server::Http;

use api::{ApiServer, Router};
use keychain::Keychain;
use handlers::CoinbaseHandler;
use receiver::WalletReceiver;
use types::WalletConfig;
use util::LOGGER;

pub fn start_rest_apis(wallet_config: WalletConfig, keychain: Keychain) {

	let receive_tx_handler = WalletReceiver {
		config: wallet_config.clone(),
		keychain: keychain.clone(),
	};
	let coinbase_handler = CoinbaseHandler {
		config: wallet_config.clone(),
		keychain: keychain.clone(),
	};

	let router = router!(
		post "/v1/receive/transaction" => receive_tx_handler,
		post "/v1/receive/coinbase" => coinbase_handler,
	);

	let apis = ApiServer::new(router.unwrap());
	info!(
		LOGGER,
		"Starting the Grin wallet API server at {}.",
		wallet_config.api_listen_addr()
	);

	let socket_addr = wallet_config.api_listen_addr().parse().unwrap(); 
	let server = Http::new().bind(&socket_addr, apis).unwrap();
	info!(
		LOGGER,
		"The Grin wallet API server is listening on http://{}.",
		server.local_addr().unwrap()
	);
	server.run().unwrap();
}
