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

//! Controller for wallet.. instantiates and handles listeners (or single-run
//! invocations) as needed.
//! Still experimental
use api::ApiServer;
use std::sync::{Arc, Mutex};

use bodyparser;
use iron::Handler;
use iron::prelude::*;
use iron::status;
use serde_json;

use failure::Fail;

use libwallet::Error;
use libwallet::api::{APIOwner, APIStranger};
use libwallet::types::WalletBackend;

use util::LOGGER;

/// Instantiate wallet Owner API for a single-use (command line) call
/// Return a function containing a loaded API context to call
pub fn owner_single_use<F, T>(wallet: &mut T, f: F) -> Result<(), Error>
where
	T: WalletBackend,
	F: FnOnce(&mut APIOwner<T>) -> Result<(), Error>,
{
	wallet.open_with_credentials()?;
	f(&mut APIOwner::new(wallet))?;
	wallet.close()?;
	Ok(())
}

/// Instantiate wallet Stranger API for a single-use (command line) call
/// Return a function containing a loaded API context to call
pub fn stranger_single_use<F, T>(wallet: &mut T, f: F) -> Result<(), Error>
where
	T: WalletBackend,
	F: FnOnce(&mut APIStranger<T>) -> Result<(), Error>,
{
	wallet.open_with_credentials()?;
	f(&mut APIStranger::new(wallet))?;
	wallet.close()?;
	Ok(())
}
/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn owner_listener<T>(wallet: T) -> Result<(), Error>
where
	T: WalletBackend,
	OwnerAPIHandler<T>: Handler,
{
	let api_handler = OwnerAPIHandler {
		wallet: Arc::new(Mutex::new(wallet)),
	};

	let router = router!(
		receive_tx: get "/wallet/owner" => api_handler,
	);

	let mut apis = ApiServer::new("/v1".to_string());
	apis.register_handler(router);
	match apis.start("127.0.0.1:13420") {
		Err(e) => error!(
			LOGGER,
			"Failed to start Grin wallet owner API listener: {}.", e
		),
		Ok(_) => info!(LOGGER, "Grin wallet owner API started at 127.0.0.1:13420"),
	};
	Ok(())
}

pub struct OwnerAPIHandler<T>
where
	T: WalletBackend,
{
	pub wallet: Arc<Mutex<T>>,
}

impl<T> OwnerAPIHandler<T>
where
	T: WalletBackend,
{
	fn handle_request(&self, _req: &mut Request, _api: &mut APIOwner<T>) -> Result<(), Error> {
		Ok(())
	}
}

impl<T> Handler for OwnerAPIHandler<T>
where
	T: WalletBackend + Send + Sync + 'static,
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		println!("handling");
		// every request should open with stored credentials,
		// do its thing and then de-init whatever secrets have been
		// stored
		let mut wallet = self.wallet.lock().unwrap();
		wallet
			.open_with_credentials()
			.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?;
		let mut api = APIOwner::new(&mut *wallet);
		self.handle_request(req, &mut api)
			.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?;
		api.wallet
			.close()
			.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?;
		Ok(Response::with((status::Ok, "")))
		//Ok(Response::with((status::BadRequest, "")))
			/*if let Ok(json) = serde_json::to_string(&coinbase) {
				Ok(Response::with((status::Ok, json)))
			} else {
				Ok(Response::with((status::BadRequest, "")))
			}
		} else {
			Ok(Response::with((status::BadRequest, "")))
		}*/	}
}
