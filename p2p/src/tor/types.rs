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
use crate::tor::process::types::{SenderIsolation, TorProcess};

use grin_util::secp::SecretKey;

use ed25519_dalek::hazmat::ExpandedSecretKey;
use hyper::StatusCode;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, TryLockError};

#[derive(Debug, thiserror::Error, Clone, Eq, PartialEq)]
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
#[derive(Debug, thiserror::Error, Clone, Eq, PartialEq)]
pub enum PostError {
	/// Request was not sent
	#[error("Not sent: {0}")]
	NotSent(String),
	/// Request may have been sent
	#[error("Possibly sent: {0}")]
	PossiblySent(String, Option<StatusCode>, Option<String>),
}

/// Onion service status
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ServiceStatus {
	Starting,
	/// External Tor has completed bootstrap
	Bootstrapped,
	Running,
	Degraded,
	Recovering,
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

type ProcessSlot = Arc<Mutex<Option<TorProcess>>>;

/// Statuses on onion services, where key is onion address.
pub(crate) static SERVICE_STATUS: LazyLock<Mutex<HashMap<String, Arc<Mutex<ServiceStatus>>>>> =
	LazyLock::new(|| Mutex::new(HashMap::new()));
/// External Tor sender
static RUNNING_PROCESS_SENDER: LazyLock<Mutex<Option<TorProcess>>> =
	LazyLock::new(|| Mutex::new(None));
/// Service slots serialize start and stop for each onion address
static RUNNING_PROCESS_SERVICES: LazyLock<Mutex<HashMap<String, ProcessSlot>>> =
	LazyLock::new(|| Mutex::new(HashMap::new()));

/// Start integrated Tor client or external process to send
/// Reuse a live sender with the same configuration
/// Existing outbound proxies do not start a sender
pub fn start_tor<T>(config: &T) -> Result<(), Error>
where
	T: TorConfig,
{
	if let Some(proxy) = config.outbound_proxy() {
		proxy.uri()?;
		return Ok(());
	}
	if config.use_integrated() {
		return start_tor_client(config);
	}
	let mut sender = RUNNING_PROCESS_SENDER.lock().unwrap();
	if let Some(process) = sender.as_mut() {
		if process.exit_status().map_err(process_error)?.is_none() {
			return if process.matches_sender(config) {
				Ok(())
			} else {
				Err(Error::Config(
					"Tor sender configuration changed; stop the sender first".into(),
				))
			};
		}
	}
	stop_process(&mut sender)?;
	*sender = Some(TorProcess::start_sender(config)?);
	Ok(())
}

pub(super) fn sender_isolation<T: TorConfig>(config: &T) -> Option<SenderIsolation> {
	let mut sender = RUNNING_PROCESS_SENDER.try_lock().ok()?;
	let process = sender.as_mut()?;
	if !process.matches_sender(config) || process.exit_status().ok()?.is_some() {
		return None;
	}
	Some(process.sender_isolation.clone())
}

/// Stop the external Tor sender and wait for its process and readers
/// Integrated clients remain shared with listeners and requests
pub fn stop_tor() -> Result<(), Error> {
	stop_process(&mut RUNNING_PROCESS_SENDER.lock().unwrap())
}

fn process_error(error: crate::tor::process::types::Error) -> Error {
	Error::Generic(format!("{:?}", error))
}

fn stop_process(process: &mut Option<TorProcess>) -> Result<(), Error> {
	if let Some(process) = process.as_mut() {
		process.kill().map_err(process_error)?;
	}
	*process = None;
	Ok(())
}

/// Stop running Tor service with provided address
pub fn stop_onion_service(onion_addr: String) -> Result<(), Error> {
	let slot = service_slot(&onion_addr);
	let mut process = slot.lock().unwrap();
	if process.is_some() {
		stop_process(&mut process)?;
	} else {
		stop_tor_service(onion_addr.clone())?;
	}
	SERVICE_STATUS.lock().unwrap().remove(&onion_addr);
	Ok(())
}

fn service_slot(address: &str) -> ProcessSlot {
	RUNNING_PROCESS_SERVICES
		.lock()
		.unwrap()
		.entry(address.to_owned())
		.or_default()
		.clone()
}

/// Start Tor service from provided key to forward requests to the given address
pub fn start_onion_service<T>(sec_key: SecretKey, addr: &str, config: &T) -> Result<(), Error>
where
	T: TorConfig,
{
	let onion_address = service_address(&sec_key)?;
	let slot = service_slot(&onion_address);
	let mut process = slot.lock().unwrap();
	if let Some(process) = process.as_mut() {
		if process.exit_status().map_err(process_error)?.is_none() {
			return Err(Error::Generic(format!(
				"Tor service {} already started",
				onion_address
			)));
		}
		process.kill().map_err(process_error)?;
		SERVICE_STATUS.lock().unwrap().remove(&onion_address);
	}
	*process = None;
	let status = Arc::new(Mutex::new(ServiceStatus::Starting));
	{
		let mut statuses = SERVICE_STATUS.lock().unwrap();
		if !matches!(
			statuses
				.get(&onion_address)
				.map(|status| status.lock().unwrap().clone()),
			None | Some(ServiceStatus::Errored(_) | ServiceStatus::Stopped)
		) {
			return Err(Error::Generic(format!(
				"Tor service {} already started",
				onion_address
			)));
		}
		statuses.insert(onion_address.clone(), status.clone());
	}
	let result = if config.use_integrated() {
		start_tor_service(sec_key, addr, config)
	} else {
		TorProcess::start_listener(sec_key, addr, config).map(|started| *process = Some(started))
	};
	let mut status = status.lock().unwrap();
	match &result {
		Ok(()) if !config.use_integrated() => {
			*status = ServiceStatus::Bootstrapped;
		}
		Err(error) => {
			*status = ServiceStatus::Errored(error.clone());
		}
		_ => {}
	}
	result
}

/// Get status of onion service.
pub fn onion_service_status(onion_address: String) -> ServiceStatus {
	let process = RUNNING_PROCESS_SERVICES
		.lock()
		.unwrap()
		.get(&onion_address)
		.cloned();
	if let Some(process) = process {
		match process.try_lock() {
			Ok(mut process) => {
				if let Some(process) = process.as_mut() {
					return match process.exit_status() {
						Ok(None) => ServiceStatus::Bootstrapped,
						Ok(Some(status)) => ServiceStatus::Errored(Error::Generic(format!(
							"Tor process exited: {}",
							status
						))),
						Err(error) => ServiceStatus::Errored(process_error(error)),
					};
				}
			}
			Err(TryLockError::WouldBlock) => {}
			Err(TryLockError::Poisoned(_)) => {
				return ServiceStatus::Errored(Error::Generic("Tor process lock poisoned".into()))
			}
		};
	}
	let service_statuses = SERVICE_STATUS.lock().unwrap();
	match service_statuses.get(&onion_address) {
		None => ServiceStatus::Stopped,
		Some(s) => s.lock().unwrap().clone(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::tor::config::{TorBridgeConfig, TorOutboundProxy, TorProxyConfig};
	use std::path::PathBuf;

	#[derive(Default)]
	struct Config {
		integrated: bool,
		dir: PathBuf,
		bridge: TorBridgeConfig,
		outbound: Option<TorOutboundProxy>,
		proxy: TorProxyConfig,
	}

	impl TorConfig for Config {
		fn use_integrated(&self) -> bool {
			self.integrated
		}
		fn request_timeout_secs(&self) -> Option<u64> {
			None
		}
		fn bootstrap_timeout_secs(&self) -> Option<u64> {
			None
		}
		fn min_circuit_timeout_ms(&self) -> Option<i32> {
			None
		}
		fn bridge_config(&self) -> TorBridgeConfig {
			self.bridge.clone()
		}
		fn proxy_config(&self) -> TorProxyConfig {
			self.proxy.clone()
		}
		fn outbound_proxy(&self) -> Option<TorOutboundProxy> {
			self.outbound.clone()
		}
		fn data_dir(&self) -> PathBuf {
			self.dir.clone()
		}
		fn socks_proxy_addr(&self) -> String {
			"127.0.0.1:9050".into()
		}
	}

	impl crate::tor::TorHttpClient<Config> for Config {
		fn tor_config(&self) -> &Config {
			self
		}
	}

	#[tokio::test]
	async fn existing_proxy() {
		use crate::tor::{TorContext, TorHttpClient};
		use tokio::io::{AsyncReadExt, AsyncWriteExt};
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let proxy = listener.local_addr().unwrap();
		let server = tokio::spawn(async move {
			let (mut stream, _) = listener.accept().await.unwrap();
			for _ in 0..2 {
				let mut headers = Vec::new();
				while !headers.ends_with(b"\r\n\r\n") {
					headers.push(stream.read_u8().await.unwrap());
				}
				assert!(headers.starts_with(b"POST http://example.onion/test HTTP/1.1\r\n"));
				stream
					.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 3\r\n\r\nbad")
					.await
					.unwrap();
			}
			assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
		});
		let dir = tempfile::tempdir().unwrap();
		let config = Config {
			integrated: true,
			dir: dir.path().join("tor"),
			outbound: Some(TorOutboundProxy::Http(format!("http://{}", proxy))),
			..Default::default()
		};
		start_tor(&config).unwrap();
		assert!(!config.dir.exists());
		let context = TorContext::new(config);
		for _ in 0..2 {
			assert!(
				matches!(context.post_tor("http://example.onion/test", None, String::new(), 3).await,
				Err(Error::Post(PostError::PossiblySent(_, Some(status), Some(body)))) if status.as_u16() == 503 && body == "bad")
			);
		}
		context.close().await;
		tokio::time::timeout(std::time::Duration::from_secs(1), server)
			.await
			.unwrap()
			.unwrap();
	}

	#[tokio::test]
	async fn reject_https() {
		use crate::tor::TorHttpClient;
		let config = Config {
			integrated: true,
			..Default::default()
		};
		assert!(matches!(
			config.check_tor("https://example.onion/").await,
			Err(Error::Post(PostError::NotSent(_)))
		));
	}

	#[test]
	fn locked_dirs() {
		use fslock_guard::LockFileGuard;
		use std::fs;
		let dir = tempfile::tempdir().unwrap();
		let config = Config {
			dir: dir.path().to_path_buf(),
			..Default::default()
		};
		let key = SecretKey::from_slice(&grin_util::secp::Secp256k1::new(), &[1; 32]).unwrap();
		let address = service_address(&key).unwrap();
		for path in [
			config.dir.join("sender"),
			config.dir.join("listener").join(&address),
		] {
			fs::create_dir_all(&path).unwrap();
			fs::write(path.join("torrc"), "existing config").unwrap();
			let _lock = LockFileGuard::lock(path.join("lock")).unwrap();
			let result = if path.ends_with("sender") {
				start_tor(&config)
			} else {
				start_onion_service(key.clone(), "127.0.0.1:3415", &config)
			};
			assert!(matches!(result, Err(Error::Config(_))));
			assert_eq!(
				fs::read_to_string(path.join("torrc")).unwrap(),
				"existing config"
			);
			assert!(!path.join("onion_service_addresses").exists());
			assert!(!path.join("pid").exists());
		}
		stop_onion_service(address).unwrap();
	}

	#[test]
	fn stale_status() {
		let config = Config {
			proxy: TorProxyConfig {
				allowed_port: Some(vec![]),
				..Default::default()
			},
			..Default::default()
		};
		let key = SecretKey::from_slice(&grin_util::secp::Secp256k1::new(), &[2; 32]).unwrap();
		let address = service_address(&key).unwrap();
		assert!(start_onion_service(key.clone(), "127.0.0.1:3415", &config).is_err());
		let old = SERVICE_STATUS.lock().unwrap()[&address].clone();
		assert!(start_onion_service(key, "127.0.0.1:3415", &config).is_err());
		*old.lock().unwrap() = ServiceStatus::Running;
		assert!(matches!(
			onion_service_status(address.clone()),
			ServiceStatus::Errored(_)
		));
		stop_onion_service(address.clone()).unwrap();
		*old.lock().unwrap() = ServiceStatus::Starting;
		assert!(matches!(
			onion_service_status(address),
			ServiceStatus::Stopped
		));
	}

	#[test]
	fn reject_empty_ports() {
		let dir = tempfile::tempdir().unwrap();
		let config = Config {
			dir: dir.path().join("tor"),
			proxy: TorProxyConfig {
				allowed_port: Some(vec![]),
				..Default::default()
			},
			..Default::default()
		};
		assert!(matches!(start_tor(&config), Err(Error::Config(_))));
		assert!(!config.dir.exists());
	}

	#[cfg(unix)]
	#[test]
	fn reject_non_utf8_dir() {
		use std::ffi::OsString;
		use std::os::unix::ffi::OsStringExt;
		let dir = tempfile::tempdir().unwrap();
		let config = Config {
			dir: dir.path().join(OsString::from_vec(b"tor-\xff".to_vec())),
			..Default::default()
		};
		assert!(matches!(start_tor(&config), Err(Error::Config(_))));
		assert!(!config.dir.exists());
	}

	struct Warnings(Mutex<Vec<String>>);
	static WARNINGS: Warnings = Warnings(Mutex::new(Vec::new()));
	impl log::Log for Warnings {
		fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
			metadata.level() == log::Level::Warn
		}
		fn log(&self, record: &log::Record<'_>) {
			if self.enabled(record.metadata()) {
				self.0.lock().unwrap().push(record.args().to_string());
			}
		}
		fn flush(&self) {}
	}

	#[test]
	fn listener_address() {
		let dir = tempfile::tempdir().unwrap();
		let config = Config {
			integrated: true,
			dir: dir.path().join("tor"),
			bridge: TorBridgeConfig {
				bridge_line: Some("invalid bridge".into()),
				..Default::default()
			},
			..Default::default()
		};
		let key = SecretKey::from_slice(&grin_util::secp::Secp256k1::new(), &[3; 32]).unwrap();
		let result = start_onion_service(key, "invalid", &config);
		assert!(
			matches!(result, Err(Error::Config(message)) if message == "Invalid Tor listener address")
		);
		assert!(!config.dir.exists());
	}

	#[test]
	fn ignored_proxy_warning() {
		// Keep the global logger separate from other unit tests
		if std::env::var_os("GRIN_TOR_PROXY_WARNING_TEST").is_none() {
			let output = std::process::Command::new(std::env::current_exe().unwrap())
				.args([
					"--exact",
					"tor::types::tests::ignored_proxy_warning",
					"--nocapture",
				])
				.env("GRIN_TOR_PROXY_WARNING_TEST", "1")
				.output()
				.unwrap();
			assert!(
				output.status.success(),
				"{}\n{}",
				String::from_utf8_lossy(&output.stdout),
				String::from_utf8_lossy(&output.stderr)
			);
			assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
			return;
		}

		log::set_logger(&WARNINGS).unwrap();
		log::set_max_level(log::LevelFilter::Warn);
		let dir = tempfile::tempdir().unwrap();
		let mut config = Config {
			integrated: true,
			dir: dir.path().join("tor"),
			// Stop before bootstrap so this test needs no network
			bridge: TorBridgeConfig {
				bridge_line: Some("invalid bridge".into()),
				..Default::default()
			},
			..Default::default()
		};
		assert!(matches!(start_tor(&config), Err(Error::Config(_))));
		assert!(WARNINGS.0.lock().unwrap().is_empty());
		config.proxy = TorProxyConfig {
			transport: Some("socks5".into()),
			address: Some("127.0.0.1:1".into()),
			username: Some("private-user".into()),
			password: Some("private-password".into()),
			..Default::default()
		};
		assert!(matches!(start_tor(&config), Err(Error::Config(_))));
		assert!(matches!(start_tor(&config), Err(Error::Config(_))));
		let messages = WARNINGS.0.lock().unwrap();
		assert_eq!(messages.len(), 1);
		assert!(messages[0].contains("proxy") && messages[0].contains("ignored"));
		assert!(!messages[0].contains("private-user") && !messages[0].contains("private-password"));
		drop(messages);
		config.proxy.address = Some("127.0.0.1:2".into());
		assert!(matches!(start_tor(&config), Err(Error::Config(_))));
		assert_eq!(WARNINGS.0.lock().unwrap().len(), 2);
		let proxy = std::mem::take(&mut config.proxy);
		assert!(matches!(start_tor(&config), Err(Error::Config(_))));
		assert_eq!(WARNINGS.0.lock().unwrap().len(), 2);
		config.proxy = proxy;
		assert!(matches!(start_tor(&config), Err(Error::Config(_))));
		assert_eq!(WARNINGS.0.lock().unwrap().len(), 3);
		assert!(!config.dir.exists());
	}
}
