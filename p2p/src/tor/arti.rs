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

use crate::tor::config::TorConfig;
use crate::tor::ov3::OnionV3Address;
use crate::tor::types::{exp_sec_key_bytes, Error, PostError};
use arti_client::config::pt::TransportConfigBuilder;
use arti_client::config::{BridgeConfigBuilder, TorClientConfigBuilder};
use arti_client::{TorClient, TorClientConfig};
use bytes::Bytes;
use ed25519_dalek::hazmat::ExpandedSecretKey;
use ed25519_dalek::Digest;
use fs_mistrust::Mistrust;
use fslock_guard::LockFileGuard;
use grin_util::secp::SecretKey;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Uri};
use hyper_util::rt::TokioIo;
use lazy_static::lazy_static;
use serde::Serialize;
use sha2::Sha512;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
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
use tor_rtcompat::{SleepProviderExt, ToplevelBlockOn};

struct TorService {
	service: Arc<RunningOnionService>,
	proxy: Arc<OnionServiceReverseProxy>,
	worker: JoinHandle<()>,
	state_lock: PathBuf,
}

type ArtiClientConfig = (Arc<TorClient<TokioNativeTlsRuntime>>, TorClientConfig);

lazy_static! {
	/// Arti Tokio runtime.
	static ref ARTI_RUNTIME: LazyLock<Mutex<Option<ArtiRuntimeWrapper>>> =
		LazyLock::new(|| Mutex::new(ArtiRuntimeWrapper::create().ok()));
	/// Arti client and config.
	static ref ARTI_CLIENT_CONFIG: LazyLock<Mutex<Option<ArtiClientConfig>>> =
		LazyLock::new(|| Mutex::new(None));
	/// Running services, where key is onion address.
	static ref ARTI_PROXY_SERVICES: LazyLock<Mutex<HashMap<String, TorService>>> =
		LazyLock::new(|| Mutex::new(HashMap::new()));
}

/// Arti client runtime wrapper.
#[derive(Clone)]
struct ArtiRuntimeWrapper {
	runtime: TokioNativeTlsRuntime,
}

impl ArtiRuntimeWrapper {
	fn create() -> Result<ArtiRuntimeWrapper, std::io::Error> {
		Ok(Self {
			runtime: TokioNativeTlsRuntime::create()?,
		})
	}
}

/// Get Tor client runtime.
fn runtime() -> Result<TokioNativeTlsRuntime, Error> {
	let mut runtime = ARTI_RUNTIME.lock().unwrap();
	let r = match runtime.as_ref() {
		None => runtime
			.insert(ArtiRuntimeWrapper::create().map_err(|e| Error::Generic(format!("{:?}", e)))?),
		Some(r) => r,
	};
	Ok(r.runtime.clone())
}

/// Stop running Tor service by onion address as key.
pub fn stop_tor_service(onion_addr: String) -> Result<(), Error> {
	let service = ARTI_PROXY_SERVICES.lock().unwrap().remove(&onion_addr);
	let Some(service) = service else {
		error!("Service {} to stop was not found", onion_addr);
		return Ok(());
	};
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
fn state_cache_paths(tor_dir: String) -> (PathBuf, PathBuf) {
	let mut tor_dir = PathBuf::from(tor_dir);
	tor_dir.push("arti");
	let state_path = tor_dir.join("state");
	let cache_path = tor_dir.join("cache");
	(state_path, cache_path)
}

/// Start Tor service from provided key.
pub fn start_tor_service<T>(
	key: SecretKey,
	addr: &str,
	tor_dir: String,
	config: &T,
) -> Result<(), Error>
where
	T: TorConfig,
{
	info!("Starting integrated Tor listener.");

	let (state_path, cache_path) = state_cache_paths(tor_dir);
	let (client, config) = init_client(&state_path, &cache_path, config)?;

	// Add service key to keystore.
	let onion_address =
		OnionV3Address::from_private(&key.0).map_err(|e| Error::Config(format!("{:?}", e)))?;
	let hs = HsNickname::new(onion_address.to_string())
		.map_err(|e| Error::Config(format!("{:?}", e)))?;
	let keystore_path = Path::new(&state_path).join("keystore");
	// Arti 0.44 uses this lock to finish stopping a service before it can start again.
	let state_lock = state_path
		.join("hss")
		.join(format!("{}.lock", onion_address));
	add_service_key(config.fs_mistrust(), &key, &hs, keystore_path)?;

	// Launch Onion service.
	let service_config = OnionServiceConfigBuilder::default()
		.nickname(hs.clone())
		.build()
		.map_err(|e| Error::Config(format!("{:?}", e)))?;
	let (service, proxy, worker) = match client.launch_onion_service(service_config) {
		Ok(res) => {
			if let Some((service, mut request)) = res {
				let addr: SocketAddr = addr
					.parse()
					.map_err(|e| Error::Generic(format!("{:?}", e)))?;
				let proxy = create_service_proxy(addr)?;
				let c = client.clone();
				let p = proxy.clone();
				// Launch service proxy.
				let worker = thread::spawn(move || {
					c.clone().runtime().block_on(async move {
						loop {
							match run_service_proxy(p.clone(), &mut request, hs.clone()).await {
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
				});
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
pub fn start_tor_client<T>(config: T, tor_dir: String) -> Result<(), Error>
where
	T: TorConfig,
{
	info!("Starting integrated Tor client");
	let (state_path, cache_path) = state_cache_paths(tor_dir);
	let (_, _) = init_client(&state_path, &cache_path, &config)?;
	Ok(())
}

/// Build an origin-form JSON POST request
fn build_post_request(json: String, url: &Uri) -> Result<Request<Full<Bytes>>, Error> {
	let authority = url
		.authority()
		.ok_or_else(|| Error::Generic(format!("URL {} has bad authority", url)))?;
	let request_target = url
		.path_and_query()
		.map(|path| path.as_str())
		.unwrap_or("/");

	Request::builder()
		.uri(request_target)
		.method("POST")
		.header("host", authority.as_str())
		.header("accept", "application/json")
		.header("content-type", "application/json")
		.body(Full::from(json))
		.map_err(|e| Error::Generic(format!("{:?}", e)))
}

/// Make POST request.
pub fn tor_post<IN, T>(
	config: &T,
	tor_dir: String,
	input: &IN,
	url: &str,
) -> Result<String, PostError>
where
	IN: Serialize,
	T: TorConfig,
{
	let json = serde_json::to_string(input)
		.map_err(|_| PostError::NotSent("Could not serialize data to JSON".to_owned()))?;
	let url = url.to_string();
	let url: Uri = url
		.parse()
		.map_err(|_| PostError::NotSent(format!("Bad URL: {}", url)))?;
	let host = match url.host() {
		None => {
			return Err(PostError::NotSent(format!("URL {} has bad host", url)));
		}
		Some(h) => h,
	}
	.to_string();
	let port = url.port_u16().unwrap_or(80);
	let request = build_post_request(json, &url)
		.map_err(|e| PostError::NotSent(format!("Cannot build request: {}", e)))?;
	let timeout = config.request_timeout();
	let (state_path, cache_path) = state_cache_paths(tor_dir);
	let (client, _) = init_client(&state_path, &cache_path, config)
		.map_err(|e| PostError::NotSent(format!("Cannot init client: {}", e)))?;
	let res: Result<String, PostError> = thread::spawn(move || {
		let c = client.clone();
		client.runtime().block_on(async move {
			let send_started = Arc::new(AtomicBool::new(false));
			let send_started_in_request = send_started.clone();
			let runtime = c.runtime().clone();
			let res = runtime
				.timeout(timeout, async move {
					let stream = c
						.connect((host, port))
						.await
						.map_err(|e| PostError::NotSent(format!("Connection: {:?}", e)))?;
					let (mut request_sender, connection) =
						hyper::client::conn::http1::handshake(TokioIo::new(stream))
							.await
							.map_err(|e| PostError::NotSent(format!("Handshake: {:?}", e)))?;

					// Spawn a task to poll the connection and drive the HTTP state.
					tokio::spawn(async move {
						if let Err(e) = connection.await {
							error!("Tor connection error: {}", e);
						}
					});

					send_started_in_request.store(true, Ordering::Release);
					let resp = request_sender
						.send_request(request)
						.await
						.map_err(|e| PostError::PossiblySent(format!("{:?}", e)))?;

					let status = resp.status();
					let body_resp = resp
						.into_body()
						.collect()
						.await
						.map_err(|e| PostError::PossiblySent(format!("{:?}", e)))?;
					let body = body_resp.to_bytes().into();
					let body_text = String::from_utf8(body)
						.map_err(|e| PostError::PossiblySent(format!("{:?}", e)))?;
					if !status.is_success() {
						return Err(PostError::PossiblySent(format!(
							"HTTP request failed with status {}: {}",
							status, body_text
						)));
					}
					Ok(body_text)
				})
				.await;
			match res {
				Err(e) if send_started.load(Ordering::Acquire) => {
					Err(PostError::PossiblySent(format!("{:?}", e)))
				}
				Err(e) => Err(PostError::NotSent(format!("{:?}", e))),
				Ok(body) => Ok(body),
			}
		})
	})
	.join()
	.unwrap_or_else(|e| Err(PostError::PossiblySent(format!("{:?}", e))))?;
	res
}

/// Create Tor client.
fn init_client<T>(
	state_path: &PathBuf,
	cache_path: &PathBuf,
	config: &T,
) -> Result<(Arc<TorClient<TokioNativeTlsRuntime>>, TorClientConfig), Error>
where
	T: TorConfig,
{
	let mut builder = TorClientConfigBuilder::from_directories(state_path, cache_path);
	builder.address_filter().allow_onion_addrs(true);
	// Circuit timeout minimum applies to listeners and sends too
	builder
		.override_net_params()
		.insert("cbtmintimeout".into(), 60_000);

	// Configure bridge.
	let bridge_config = config.bridge_config();
	if let Some(bridge_line) = bridge_config.bridge_line.as_ref() {
		let bridge: BridgeConfigBuilder = bridge_line
			.parse()
			.map_err(|e| Error::Config(format!("{:?}", e)))?;
		builder.bridges().bridges().push(bridge.clone());
		match bridge.get_transport() {
			None => {
				return Err(Error::Config(format!(
					"No transport found at {}",
					bridge_line
				)))
			}
			Some(t) => {
				// Now configure bridge transport. (Requires the "pt-client" feature)
				let bin_path = bridge_config
					.bridge_bin_path
					.clone()
					.unwrap_or(t.to_owned());
				let mut transport = TransportConfigBuilder::default();
				transport
					.protocols(vec![t
						.parse()
						.map_err(|e| Error::Config(format!("{:?}", e)))?])
					.path(CfgPath::new(bin_path))
					.run_on_startup(true);
				builder.bridges().transports().push(transport);
			}
		}
	}
	let arti_config = builder
		.build()
		.map_err(|e| Error::Config(format!("{:?}", e)))?;

	// Return existing client if config was not changed.
	let mut cached_client_config = ARTI_CLIENT_CONFIG.lock().unwrap();
	if let Some((client, c)) = cached_client_config.as_ref() {
		if c == &arti_config {
			debug!("Reusing Arti Tor client from global state.");
			return Ok((client.clone(), c.clone()));
		} else {
			debug!("Tor config changed, rebuild client.");
			*cached_client_config = None;
		}
	}
	let res = launch_client(arti_config.clone(), config);
	match res {
		Ok(client) => {
			cached_client_config.replace((client.clone(), arti_config.clone()));
			Ok((client, arti_config))
		}
		Err(e) => Err(e),
	}
}

/// Launch tor client from provided configuration.
fn launch_client<T>(
	client_config: TorClientConfig,
	config: &T,
) -> Result<Arc<TorClient<TokioNativeTlsRuntime>>, Error>
where
	T: TorConfig,
{
	let r = runtime()?;
	let client = TorClient::with_runtime(r)
		.config(client_config)
		.create_unbootstrapped()
		.map_err(|e| Error::Generic(format!("{:?}", e)))?;
	let c = client.clone();
	let timeout = config.bootstrap_timeout();
	let res = client.runtime().block_on(async move {
		let bootstrap = async || {
			return match c.bootstrap().await {
				Ok(_) => {
					let mut percent = 0.0;
					let mut prev_percent = 0.0;
					while percent < 1.0 {
						percent = c.bootstrap_status().as_frac();
						if percent != prev_percent {
							info!("Starting Tor {}%", percent * 100.0);
						}
						prev_percent = percent;
						tokio::time::sleep(Duration::from_millis(1000)).await;
					}
					info!("Tor client bootstrapped successfully");
					Ok(())
				}
				Err(e) => Err(e),
			};
		};
		match c.runtime().timeout(timeout, bootstrap()).await {
			Ok(r) => match r {
				Err(e) => Err(Error::Generic(format!("{:?}", e))),
				Ok(_) => Ok(c),
			},
			Err(e) => Err(Error::Generic(format!("{:?}", e))),
		}
	});
	res
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
	proxy: Arc<OnionServiceReverseProxy>,
	request: &mut S,
	nickname: HsNickname,
) -> Result<(), Error>
where
	S: futures::Stream<Item = tor_hsservice::RendRequest> + Unpin + Send + 'static,
{
	let runtime = runtime()?;
	proxy
		.handle_requests(runtime, nickname, request)
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
	use super::build_post_request;
	use hyper::{Method, Uri};

	#[test]
	fn builds_origin_form_json_request() {
		let url: Uri = "http://example.onion:8080/v1?test=1".parse().unwrap();
		let request = build_post_request("{}".to_string(), &url).unwrap();

		assert_eq!(request.method(), Method::POST);
		assert_eq!(request.uri(), "/v1?test=1");
		assert_eq!(request.headers()["host"], "example.onion:8080");
		assert_eq!(request.headers()["accept"], "application/json");
		assert_eq!(request.headers()["content-type"], "application/json");
	}
}
