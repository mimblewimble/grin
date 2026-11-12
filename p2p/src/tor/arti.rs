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

use crate::tor::client::HttpConnection;
use crate::tor::config::{
	request_timeout, TorConfig, TorProxyConfig, BOOTSTRAP_TIMEOUT_SECS, CONNECT_RETRY_INTERVAL,
	CONNECT_TIMEOUT, MIN_CIRCUIT_TIMEOUT_MS,
};
use crate::tor::status::spawn_status_logger;
use crate::tor::types::{exp_sec_key_bytes, Error, PostError};
use crate::tor::{service_address, SERVICE_STATUS};

use grin_util::secp::SecretKey;

use arti_client::config::pt::TransportConfigBuilder;
use arti_client::config::{Reconfigure, TorClientConfigBuilder};
use arti_client::{TorClient, TorClientConfig};
use ed25519_dalek::hazmat::ExpandedSecretKey;
use ed25519_dalek::Digest;
use fs_mistrust::Mistrust;
use fslock_guard::LockFileGuard;
use futures::task::SpawnExt;
use hyper::Uri;
use hyper_util::rt::TokioIo;
use sha2::Sha512;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tor_hscrypto::pk::{HsIdKey, HsIdKeypair};
use tor_hsrproxy::config::{
	Encapsulation, ProxyAction, ProxyConfigBuilder, ProxyPattern, ProxyRule, TargetAddr,
};
use tor_hsrproxy::OnionServiceReverseProxy;
use tor_hsservice::config::OnionServiceConfigBuilder;
use tor_hsservice::{
	HsIdKeypairSpecifier, HsIdPublicKeySpecifier, HsNickname, RunningOnionService,
};
use tor_keymgr::config::CfgPath;
use tor_keymgr::{ArtiNativeKeystore, KeyMgrBuilder, KeystoreSelector};
use tor_llcrypto::pk::ed25519::ExpandedKeypair;
use tor_rtcompat::tokio::TokioNativeTlsRuntime;
use tor_rtcompat::ToplevelBlockOn;

struct TorService {
	service: Arc<RunningOnionService>,
	proxy: Arc<OnionServiceReverseProxy>,
	worker: JoinHandle<()>,
	state_lock: PathBuf,
}

type ArtiClientConfig = (
	Arc<TorClient<TokioNativeTlsRuntime>>,
	TorClientConfig,
	Duration,
	arti_client::StreamPrefs,
	ClientRuntime,
);

/// Arti Tokio runtime.
static ARTI_RUNTIME: Mutex<Option<TokioNativeTlsRuntime>> = Mutex::new(None);
/// Arti client and config.
static ARTI_CLIENT_CONFIG: LazyLock<AsyncMutex<(Option<ArtiClientConfig>, TorProxyConfig)>> =
	LazyLock::new(|| AsyncMutex::new(Default::default()));
/// Running services, where key is onion address.
static ARTI_PROXY_SERVICES: LazyLock<Mutex<HashMap<String, TorService>>> =
	LazyLock::new(|| Mutex::new(HashMap::new()));

/// Arti client runtime wrapper.
struct ClientRuntime {
	runtime: TokioNativeTlsRuntime,
	executor: Option<tokio::runtime::Runtime>,
}

impl ClientRuntime {
	fn create() -> Result<ClientRuntime, Error> {
		let executor = tokio::runtime::Builder::new_multi_thread()
			.enable_all()
			.build()
			.map_err(|e| Error::Generic(format!("{:?}", e)))?;
		Ok(Self {
			runtime: executor.handle().clone().into(),
			executor: Some(executor),
		})
	}
}

impl Drop for ClientRuntime {
	fn drop(&mut self) {
		if let Some(executor) = self.executor.take() {
			executor.shutdown_background();
		}
	}
}

/// Get Tor client runtime.
fn runtime() -> Result<TokioNativeTlsRuntime, Error> {
	let mut runtime = ARTI_RUNTIME.lock().unwrap();
	let r = match runtime.as_ref() {
		None => runtime.insert(
			TokioNativeTlsRuntime::create().map_err(|e| Error::Generic(format!("{:?}", e)))?,
		),
		Some(r) => r,
	};
	Ok(r.clone())
}

/// Stop running Tor service by onion address as key.
pub(crate) fn stop_tor_service(onion_addr: String) -> Result<(), Error> {
	let service = ARTI_PROXY_SERVICES.lock().unwrap().remove(&onion_addr);
	let Some(service) = service else {
		error!("Service {} to stop was not found", onion_addr);
		return Ok(());
	};
	SERVICE_STATUS.lock().unwrap().remove(&onion_addr);

	let TorService {
		service,
		proxy,
		worker,
		state_lock,
	} = service;

	proxy.shutdown();
	worker
		.join()
		.map_err(|_| Error::Generic("Tor proxy thread panicked".into()))?;
	drop(service);
	drop(
		LockFileGuard::lock(state_lock)
			.map_err(|e| Error::Generic(format!("Can not stop Tor service: {}", e)))?,
	);
	Ok(())
}

/// Get state and cache data paths.
fn state_cache_paths<T>(config: &T) -> (PathBuf, PathBuf)
where
	T: TorConfig,
{
	let mut tor_dir = config.data_dir();
	tor_dir.push("arti");
	let state_path = tor_dir.join("state");
	let cache_path = tor_dir.join("cache");
	(state_path, cache_path)
}

/// Start Tor service from provided key.
pub(crate) fn start_tor_service<T>(key: SecretKey, addr: &str, config: &T) -> Result<(), Error>
where
	T: TorConfig,
{
	info!("Starting integrated Tor listener.");
	let addr: SocketAddr = addr
		.parse()
		.map_err(|_| Error::Config("Invalid Tor listener address".into()))?;

	let onion_address = service_address(&key)?;
	if ARTI_PROXY_SERVICES
		.lock()
		.unwrap()
		.contains_key(&onion_address)
	{
		return Err(Error::Config(
			"Tor service is still in use; stop it first".into(),
		));
	}
	let (state_path, cache_path) = state_cache_paths(config);
	let (client, config) = runtime()?.block_on(init_client(&state_path, &cache_path, config))?;

	// Add service key to keystore.
	let hs =
		HsNickname::new(onion_address.clone()).map_err(|e| Error::Config(format!("{:?}", e)))?;
	let keystore_path = Path::new(&state_path).join("keystore");
	// Arti uses this lock to finish stopping a service before it can start again.
	let state_lock = state_path
		.join("hss")
		.join(format!("{}.lock", onion_address));
	add_service_key(config.fs_mistrust(), &key, &hs, keystore_path)?;

	// Launch Onion service.
	let service_config = OnionServiceConfigBuilder::default()
		.nickname(hs.clone())
		.build()
		.map_err(|e| Error::Config(format!("{:?}", e)))?;
	let proxy_runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.map_err(|e| Error::Generic(format!("Can not start Tor proxy: {}", e)))?;
	let (service, proxy, worker) = match client.launch_onion_service(service_config) {
		Ok(res) => {
			if let Some((service, mut request)) = res {
				spawn_status_logger(client.runtime(), &service, onion_address.to_string())?;

				// Launch service proxy.
				let proxy = create_service_proxy(addr)?;
				let c = client.clone();
				let p = proxy.clone();
				let runtime = proxy_runtime.handle().clone().into();
				// Keep circuit timers on the client runtime
				let task = client
					.runtime()
					.spawn_with_handle(async move {
						let _client = c;
						loop {
							match run_service_proxy(&runtime, p.clone(), &mut request, hs.clone())
								.await
							{
								Ok(_) => {
									info!("Tor proxy stopped");
									break;
								}
								Err(e) => {
									error!("Tor proxy error: {:?}, restarting", e);
									tokio::time::sleep(Duration::from_millis(1000)).await;
									continue;
								}
							}
						}
					})
					.map_err(|e| Error::Generic(format!("Can not start Tor proxy: {}", e)))?;
				// Stop forwarding tasks when this worker exits
				let worker = thread::spawn(move || proxy_runtime.block_on(task));
				(service, proxy, worker)
			} else {
				return Err(Error::Generic("Can not launch onion service".to_owned()));
			}
		}
		Err(e) => return Err(Error::Generic(format!("{:?}", e))),
	};

	let mut running_services = ARTI_PROXY_SERVICES.lock().unwrap();
	running_services.insert(
		onion_address.to_string(),
		TorService {
			service,
			proxy,
			worker,
			state_lock,
		},
	);

	Ok(())
}

/// Start Tor client to send requests.
pub(crate) fn start_tor_client<T>(config: &T) -> Result<(), Error>
where
	T: TorConfig,
{
	info!("Starting integrated Tor client");
	let (state_path, cache_path) = state_cache_paths(config);
	let (_, _) = runtime()?.block_on(init_client(&state_path, &cache_path, config))?;
	Ok(())
}

pub(super) async fn connect<T: TorConfig>(
	config: &T,
	url: &Uri,
	prefs: Option<&mut arti_client::StreamPrefs>,
	deadline: tokio::time::Instant,
) -> Result<HttpConnection, PostError> {
	if url.scheme_str() != Some("http") {
		return Err(PostError::NotSent(
			"Integrated Tor only supports HTTP requests".to_string(),
		));
	}
	let (state_path, cache_path) = state_cache_paths(config);
	let (client, _) = init_client(&state_path, &cache_path, config)
		.await
		.map_err(|e| PostError::NotSent(format!("Cannot init client: {}", e)))?;
	let mut selected = match &prefs {
		Some(prefs) => (*prefs).clone(),
		None => {
			let state = ARTI_CLIENT_CONFIG.lock().await;
			state.0.as_ref().unwrap().3.clone()
		}
	};
	let target = (url.host().unwrap(), url.port_u16().unwrap_or(80));
	let stream = loop {
		let start = tokio::time::Instant::now();
		if start >= deadline {
			return Err(PostError::NotSent("Connection timed out".into()));
		}
		let until = deadline.min(start + CONNECT_TIMEOUT);
		let result = tokio::time::timeout_at(until, client.connect_with_prefs(target, &selected))
			.await
			.map_err(|_| PostError::NotSent("Connection timed out".into()))
			.and_then(|result| {
				result.map_err(|e| PostError::NotSent(format!("Connection: {:?}", e)))
			});
		match result {
			Ok(stream) => break stream,
			Err(e) if tokio::time::Instant::now() >= deadline => return Err(e),
			Err(_) => {
				selected.new_isolation_group();
			}
		}
		tokio::time::sleep_until(deadline.min(start + CONNECT_RETRY_INTERVAL)).await;
	};
	if let Some(prefs) = prefs {
		*prefs = selected;
	} else {
		ARTI_CLIENT_CONFIG.lock().await.0.as_mut().unwrap().3 = selected;
	}
	let HttpConnection { sender, driver } = HttpConnection::open(TokioIo::new(stream)).await?;
	Ok(HttpConnection {
		sender,
		driver: Box::pin(async move {
			// Keep the client until the connection closes
			let _client = client;
			driver.await
		}),
	})
}

fn configure_bridges(
	builder: &mut TorClientConfigBuilder,
	config: crate::tor::config::TorBridgeConfig,
) -> Result<(), Error> {
	let mut transports = HashMap::new();
	let (bridge_builders, _) = config.parse()?;
	for b in bridge_builders {
		builder.bridges().bridges().push(b.clone());

		if let Some(protocol) = b.get_transport().filter(|p| !p.is_empty()) {
			let args = config.arguments(protocol)?;
			let path = config.binary(protocol)?;
			if let Some(previous) =
				transports.insert(protocol.to_string(), (path.clone(), args.clone()))
			{
				if previous != (path.clone(), args.clone()) {
					return Err(Error::Config(
						"Conflicting bridge binaries or arguments".into(),
					));
				}
			} else {
				let mut transport = TransportConfigBuilder::default();
				transport
					.protocols(vec![protocol
						.parse()
						.map_err(|e| Error::Config(format!("{:?}", e)))?])
					.path(CfgPath::new_literal(path))
					.arguments(args)
					.run_on_startup(true);
				builder.bridges().transports().push(transport);
			}
		}
	}
	Ok(())
}

/// Create Tor client.
async fn init_client<T>(
	state_path: &PathBuf,
	cache_path: &PathBuf,
	config: &T,
) -> Result<(Arc<TorClient<TokioNativeTlsRuntime>>, TorClientConfig), Error>
where
	T: TorConfig,
{
	let mut state = ARTI_CLIENT_CONFIG.lock().await;
	let (cached_client_config, proxy_config) = &mut *state;
	if proxy_config != &config.proxy_config() {
		*proxy_config = config.proxy_config().clone();
		if proxy_config != &Default::default() {
			warn!("Integrated Tor proxy configuration is ignored");
		}
	}
	let mut builder = TorClientConfigBuilder::from_directories(state_path, cache_path);
	builder.address_filter().allow_onion_addrs(true);
	// Circuit timeout minimum applies to listeners and sends too
	builder.override_net_params().insert(
		"cbtmintimeout".into(),
		config
			.min_circuit_timeout_ms()
			.unwrap_or(MIN_CIRCUIT_TIMEOUT_MS),
	);

	configure_bridges(&mut builder, config.bridge_config())?;

	let arti_config = builder
		.build()
		.map_err(|e| Error::Config(format!("{:?}", e)))?;

	// Return existing client if config was not changed.
	let connect_timeout = request_timeout(config);
	builder.stream_timeouts().connect_timeout(connect_timeout);
	let client_config = builder
		.build()
		.map_err(|e| Error::Config(format!("{:?}", e)))?;
	if let Some((client, c, timeout, _, _)) = cached_client_config.as_mut() {
		if c == &arti_config {
			// A shorter request must not shorten another request's connect budget
			if connect_timeout > *timeout {
				client
					.reconfigure(&client_config, Reconfigure::AllOrNothing)
					.map_err(|e| Error::Config(format!("{:?}", e)))?;
				*timeout = connect_timeout;
			}
			debug!("Reusing Arti Tor client from global state.");
			return Ok((client.clone(), c.clone()));
		} else {
			if Arc::strong_count(client) > 1 {
				return Err(Error::Config(
					"Stop using the Tor client before changing its configuration".into(),
				));
			}
			debug!("Tor config changed, reconfigure client.");
			client
				.reconfigure(&client_config, Reconfigure::AllOrNothing)
				.map_err(|e| Error::Config(format!("{:?}", e)))?;
			return Ok((client.clone(), c.clone()));
		}
	}
	let (client, runtime) = launch_client(client_config, config).await?;
	cached_client_config.replace((
		client.clone(),
		arti_config.clone(),
		connect_timeout,
		Default::default(),
		runtime,
	));
	Ok((client, arti_config))
}

/// Launch tor client from provided configuration.
async fn launch_client<T>(
	client_config: TorClientConfig,
	config: &T,
) -> Result<(Arc<TorClient<TokioNativeTlsRuntime>>, ClientRuntime), Error>
where
	T: TorConfig,
{
	let timeout = Duration::from_secs(
		config
			.bootstrap_timeout_secs()
			.filter(|s| *s > 0)
			.unwrap_or(BOOTSTRAP_TIMEOUT_SECS),
	);
	tokio::time::timeout(timeout, async {
		let runtime = tokio::task::spawn_blocking(ClientRuntime::create)
			.await
			.map_err(|e| Error::Generic(format!("{:?}", e)))??;
		let client = TorClient::with_runtime(runtime.runtime.clone())
			.config(client_config)
			.create_unbootstrapped_async()
			.await
			.map_err(|e| Error::Generic(format!("{:?}", e)))?;
		client
			.bootstrap()
			.await
			.map_err(|e| Error::Generic(format!("{:?}", e)))?;
		info!("Tor client bootstrapped successfully");
		Ok((client, runtime))
	})
	.await
	.map_err(|e| Error::Generic(format!("{:?}", e)))?
}

/// Create Onion service proxy.
fn create_service_proxy(addr: SocketAddr) -> Result<Arc<OnionServiceReverseProxy>, Error> {
	// Setup proxy to forward request from Tor address to local address.
	let proxy_rule = ProxyRule::new(
		ProxyPattern::one_port(80).map_err(|e| Error::Config(format!("{}", e)))?,
		ProxyAction::Forward(Encapsulation::Simple, TargetAddr::Inet(addr)),
	);
	let mut proxy_cfg_builder = ProxyConfigBuilder::default();
	proxy_cfg_builder.set_proxy_ports(vec![proxy_rule]);
	let proxy_cfg = proxy_cfg_builder
		.build()
		.map_err(|e| Error::Config(format!("{}", e)))?;
	let proxy = OnionServiceReverseProxy::new(proxy_cfg);
	Ok(proxy)
}

/// Launch service proxy.
async fn run_service_proxy<S>(
	runtime: &TokioNativeTlsRuntime,
	proxy: Arc<OnionServiceReverseProxy>,
	request: &mut S,
	nickname: HsNickname,
) -> Result<(), Error>
where
	S: futures::Stream<Item = tor_hsservice::RendRequest> + Unpin + Send + 'static,
{
	proxy
		.handle_requests(runtime.clone(), nickname, request)
		.await
		.map_err(|e| Error::Generic(format!("{:?}", e)))?;
	Ok(())
}

/// Save Onion service key to keystore.
fn add_service_key(
	mistrust: &Mistrust,
	key: &SecretKey,
	hs_nickname: &HsNickname,
	path: PathBuf,
) -> Result<(), Error> {
	let arti_store = ArtiNativeKeystore::from_path_and_mistrust(path, mistrust)
		.map_err(|e| Error::Generic(format!("{}", e)))?;

	let key_manager = KeyMgrBuilder::default()
		.primary_store(Box::new(arti_store))
		.build()
		.map_err(|e| Error::Generic(format!("{}", e)))?;

	let expanded_sk =
		ExpandedSecretKey::from_bytes(Sha512::default().chain_update(key).finalize().as_ref());
	let sk_bytes = exp_sec_key_bytes(expanded_sk);
	match ExpandedKeypair::from_secret_key_bytes(sk_bytes) {
		None => {
			return Err(Error::Generic(
				"Hidden service key can not be created".into(),
			))
		}
		Some(expanded_kp) => {
			key_manager
				.insert(
					HsIdKey::from(*expanded_kp.public()),
					&HsIdPublicKeySpecifier::new(hs_nickname.clone()),
					KeystoreSelector::Primary,
					true,
				)
				.map_err(|e| Error::Generic(format!("{}", e)))?;
			key_manager
				.insert(
					HsIdKeypair::from(expanded_kp),
					&HsIdKeypairSpecifier::new(hs_nickname.clone()),
					KeystoreSelector::Primary,
					true,
				)
				.map_err(|e| Error::Generic(format!("{}", e)))?;
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::tor::config::TorBridgeConfig;

	#[tokio::test]
	async fn runtime_drop() {
		let runtime = tokio::task::spawn_blocking(ClientRuntime::create)
			.await
			.unwrap()
			.unwrap();
		let handle = runtime.runtime.clone();
		let (started_tx, started_rx) = tokio::sync::oneshot::channel();
		let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel::<()>();
		handle
			.spawn(async move {
				let _stopped = stopped_tx;
				started_tx.send(()).unwrap();
				std::future::pending::<()>().await;
			})
			.unwrap();
		started_rx.await.unwrap();
		drop(runtime);
		assert!(tokio::time::timeout(Duration::from_secs(2), stopped_rx)
			.await
			.unwrap()
			.is_err());
	}

	#[test]
	fn bridge_list() {
		let bridge = TorBridgeConfig {
			bridge_line: Some("webtunnel 192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567 url=https://example.com/bridge
webtunnel 192.0.3.1:443 0123456789ABCDEF0123456789ABCDEF01234567 url=https://example2.com/bridge".into()),
			bridge_bin_path: Some("/tmp/bridge client".into()),
			client_args: vec!["--test".into()],
		};
		let mut builder = TorClientConfigBuilder::default();
		configure_bridges(&mut builder, bridge.clone()).unwrap();
		assert_eq!(builder.bridges().bridges().len(), 2);
		assert_eq!(builder.bridges().transports().len(), 1);

		let mut multiple_protocols = bridge.clone();
		multiple_protocols.bridge_line = Some("webtunnel 192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567 url=https://example.com/bridge
snowflake 192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567 url=http://127.0.0.1:9/ ice=stun:127.0.0.1:9".into());
		assert!(configure_bridges(&mut builder, bridge.clone()).is_ok());

		let mut bad_protocol = bridge.clone();
		bad_protocol.bridge_line = Some("192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567 url=https://example.com/bridge
snowflake 192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567 url=http://127.0.0.1:9/ ice=stun:127.0.0.1:9".into());
		assert!(configure_bridges(&mut builder, bridge).is_ok());
	}
}
