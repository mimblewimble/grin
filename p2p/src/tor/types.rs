// Copyright 2026 The Grin Developers
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

use crate::tor::arti::{start_tor_client, start_tor_service, stop_tor_service};
use crate::tor::config::TorConfig;
use crate::tor::ov3::OnionV3Address;
use crate::tor::process::types::TorProcess;

use grin_util::secp::SecretKey;

use ed25519_dalek::hazmat::ExpandedSecretKey;
use grin_util::StopState;
use hyper::StatusCode;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, thiserror::Error, Clone)]
pub enum Error {
	/// Generic Tor error
	#[error("Tor Process Error: {0}")]
	Generic(String),
	/// Tor configuration error
	#[error("Tor Config Error: {0}")]
	Config(String),
	/// POST HTTP request error.
	#[error("Tor POST Error: {0}")]
	Post(PostError),
}

/// Failure from a POST HTTP request over Tor, classified by whether delivery may
/// have started.
#[derive(Debug, thiserror::Error, Clone)]
pub enum PostError {
	/// The failure happened before handing the request to Hyper.
	#[error("Not sent: {0}")]
	NotSent(String),
	/// Hyper started sending the request, so delivery cannot be ruled out.
	#[error("Possibly sent: {0}")]
	PossiblySent(String, Option<StatusCode>, Option<String>),
}

/// Onion service status
#[derive(Debug, Clone)]
pub enum ServiceStatus {
	Starting,
	Running,
	Errored(Error),
	Stopped,
}

/// Convert expanded secret key to byte array (works for both external Tor process and Arti).
pub(crate) fn exp_sec_key_bytes(expanded_sk: ExpandedSecretKey) -> [u8; 64] {
	let mut sk_bytes = [0_u8; 64];
	sk_bytes[0..32].copy_from_slice(&expanded_sk.scalar.to_bytes());
	sk_bytes[32..64].copy_from_slice(&expanded_sk.hash_prefix);
	sk_bytes
}

/// Get onion address from key.
pub fn service_address(key: &SecretKey) -> Result<String, Error> {
	let onion_address =
		OnionV3Address::from_private(&key.0).map_err(|e| Error::Config(format!("{:?}", e)))?;
	Ok(onion_address.to_string())
}

lazy_static! {
	/// Statuses on onion services, where key is onion address.
	pub (crate) static ref SERVICE_STATUS: LazyLock<Mutex<HashMap<String, ServiceStatus>>> =
		LazyLock::new(|| Mutex::new(HashMap::new()));
	/// Running Tor sender process data dir.
	static ref RUNNING_PROCESS_DIR: LazyLock<Mutex<Option<String>>> =
		LazyLock::new(|| Mutex::new(None));
	/// Running Tor process services states, where key is onion address.
	static ref RUNNING_PROCESS_SERVICES: LazyLock<Mutex<HashMap<String, StopState>>> =
		LazyLock::new(|| Mutex::new(HashMap::new()));
}

/// Start integrated Tor client or external process to send.
/// Integrated Tor will reuse same client.
/// Process will be reused if same data dir passed in config.
pub fn start_tor<T>(config: &T) -> Result<(), Error>
where
	T: TorConfig,
{
	if config.use_integrated() {
		start_tor_client(config)?;
	} else {
		let data_dir = config
			.data_dir()
			.join("sender")
			.to_str()
			.unwrap()
			.to_string();
		let running = {
			let running_process_dir = RUNNING_PROCESS_DIR.lock().unwrap();
			running_process_dir.as_ref() == Some(&data_dir)
		};
		if running {
			return Ok(());
		}

		RUNNING_PROCESS_DIR.lock().unwrap().replace(data_dir);

		let p = TorProcess::start_sender(config)?;
		thread::spawn(move || {
			let p = p;
			loop {
				let stop_needed = {
					let running_process_dir = RUNNING_PROCESS_DIR.lock().unwrap();
					running_process_dir.as_ref() != p.working_dir.as_ref()
				};
				if stop_needed {
					drop(p);
					break;
				}
				thread::sleep(Duration::from_millis(1000));
			}
		});
	};
	Ok(())
}

/// Stop running Tor service with provided address.
pub fn stop_onion_service(onion_addr: String) -> Result<(), Error> {
	if let Some(stop_state) = RUNNING_PROCESS_SERVICES.lock().unwrap().get(&onion_addr) {
		stop_state.stop();
	} else {
		stop_tor_service(onion_addr)?;
	}
	Ok(())
}

/// Start Tor service from provided key to forward requests to
/// provided address and store listener data at provided dir for process.
pub fn start_onion_service<T>(
	sec_key: SecretKey,
	addr: &str,
	dir: PathBuf,
	config: &T,
) -> Result<(), Error>
where
	T: TorConfig,
{
	let onion_address = service_address(&sec_key)?;
	{
		let mut service_statuses = SERVICE_STATUS.lock().unwrap();
		match service_statuses.get(&onion_address) {
			None => {
				service_statuses.insert(onion_address.clone(), ServiceStatus::Starting);
			}
			Some(s) => match s {
				ServiceStatus::Errored(_) => {
					warn!("Restarting errored Tor service {}", onion_address);
				}
				_ => {
					let error = format!("Tor service {} already started", onion_address);
					warn!("{}", error);
					return Err(Error::Generic(error));
				}
			},
		}
	}
	let res: Result<(), Error> = if config.use_integrated() {
		match start_tor_service(sec_key, addr, config) {
			Ok(_) => Ok(()),
			Err(e) => {
				error!("Error starting Tor service with integrated client: {}", e);
				Err(e)
			}
		}
	} else {
		match TorProcess::start_listener(sec_key, dir, addr, config) {
			Ok(p) => {
				let address = onion_address.clone();
				thread::spawn(move || {
					let p = p;
					loop {
						let stopped = {
							if let Some(state) =
								RUNNING_PROCESS_SERVICES.lock().unwrap().get(&address)
							{
								state.is_stopped()
							} else {
								false
							}
						};
						if stopped {
							drop(p);
							break;
						}
						thread::sleep(Duration::from_millis(1000));
					}
				});
				Ok(())
			}
			Err(e) => {
				error!("Error starting Tor service with external process: {}", e);
				Err(e)
			}
		}
	};

	let mut service_statuses = SERVICE_STATUS.lock().unwrap();
	match &res {
		Ok(_) => {
			if !config.use_integrated() {
				service_statuses.insert(onion_address, ServiceStatus::Running);
			}
		}
		Err(e) => {
			service_statuses.insert(onion_address, ServiceStatus::Errored(e.clone()));
		}
	}
	res
}

/// Get status of onion service.
pub fn onion_service_status(onion_address: String) -> ServiceStatus {
	let service_statuses = SERVICE_STATUS.lock().unwrap();
	match service_statuses.get(&onion_address) {
		None => ServiceStatus::Stopped,
		Some(s) => s.clone(),
	}
}
