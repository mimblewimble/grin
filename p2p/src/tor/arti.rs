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

use crate::tor::client::read_response;
use crate::tor::config::{
	request_timeout, TorConfig, BOOTSTRAP_TIMEOUT_SECS, MIN_CIRCUIT_TIMEOUT_MS,
};
use crate::tor::types::{exp_sec_key_bytes, Error, PostError};
use crate::tor::{service_address, spawn_status_logger, SERVICE_STATUS};

use grin_util::secp::SecretKey;

use arti_client::config::pt::TransportConfigBuilder;
use arti_client::config::{BridgeConfigBuilder, Reconfigure, TorClientConfigBuilder};
use arti_client::{TorClient, TorClientConfig};
use bytes::Bytes;
use ed25519_dalek::hazmat::ExpandedSecretKey;
use ed25519_dalek::Digest;
use fs_mistrust::Mistrust;
use fslock_guard::LockFileGuard;
use http_body_util::Full;
use hyper::{Request, Response, Uri};
use hyper_util::rt::TokioIo;
use lazy_static::lazy_static;
use sha2::Sha512;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{timeout_at, Instant};
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
);

lazy_static! {
	/// Arti Tokio runtime.
	static ref ARTI_RUNTIME: LazyLock<Mutex<Option<ArtiRuntimeWrapper>>> =
		LazyLock::new(|| Mutex::new(ArtiRuntimeWrapper::create().ok()));
	/// Arti client and config.
	static ref ARTI_CLIENT_CONFIG: LazyLock<AsyncMutex<Option<ArtiClientConfig>>> =
		LazyLock::new(|| AsyncMutex::new(None));
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
	let (service, proxy, worker) = match client.launch_onion_service(service_config) {
		Ok(res) => {
			if let Some((service, mut request)) = res {
				spawn_status_logger(client.runtime(), &service, onion_address.to_string())?;

				// Launch service proxy.
				let addr: SocketAddr = addr
					.parse()
					.map_err(|e| Error::Generic(format!("{:?}", e)))?;
				let proxy = create_service_proxy(addr)?;
				let c = client.clone();
				let p = proxy.clone();
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
pub(crate) fn start_tor_client<T>(config: &T) -> Result<(), Error>
where
	T: TorConfig,
{
	info!("Starting integrated Tor client");
	let (state_path, cache_path) = state_cache_paths(config);
	let (_, _) = runtime()?.block_on(init_client(&state_path, &cache_path, config))?;
	Ok(())
}

/// Build an origin-form POST request
fn build_post_request(
	body: String,
	headers: Option<HashMap<&str, &str>>,
	url: &Uri,
) -> Result<Request<Full<Bytes>>, Error> {
	let authority = url
		.authority()
		.ok_or_else(|| Error::Generic(format!("URL {} has bad authority", url)))?;
	let request_target = url
		.path_and_query()
		.map(|path| path.as_str())
		.unwrap_or("/");

	let mut req = Request::builder().uri(request_target).method("POST");
	if let Some(headers) = headers {
		for (k, v) in headers {
			req = req.header(k, v);
		}
	}
	req.header("host", authority.as_str())
		.body(Full::from(body))
		.map_err(|e| Error::Generic(format!("{:?}", e)))
}

/// Make POST request.
pub(crate) async fn tor_post<T>(
	config: &T,
	body: String,
	headers: Option<HashMap<&str, &str>>,
	url: &str,
	response_limit: usize,
) -> Result<Response<String>, PostError>
where
	T: TorConfig,
{
	let url = url.to_string();
	let url: Uri = url
		.parse()
		.map_err(|_| PostError::NotSent(format!("Bad URL: {}", url)))?;
	if url.scheme_str() != Some("http") {
		return Err(PostError::NotSent(
			"Integrated Tor only supports HTTP requests".to_string(),
		));
	}
	let host = match url.host() {
		None => {
			return Err(PostError::NotSent(format!("URL {} has bad host", url)));
		}
		Some(h) => h,
	}
	.to_string();
	let port = url.port_u16().unwrap_or(80);
	let request = build_post_request(body, headers, &url)
		.map_err(|e| PostError::NotSent(format!("Cannot build request: {}", e)))?;
	let deadline = Instant::now() + request_timeout(config);
	let stream = timeout_at(deadline, async {
		let (state_path, cache_path) = state_cache_paths(config);
		let (client, _) = init_client(&state_path, &cache_path, config)
			.await
			.map_err(|e| PostError::NotSent(format!("Cannot init client: {}", e)))?;
		client
			.connect((host, port))
			.await
			.map_err(|e| PostError::NotSent(format!("Connection: {:?}", e)))
	})
	.await
	.map_err(|_| PostError::NotSent("Tor request timed out".into()))??;
	post_request(stream, request, response_limit, deadline).await
}

async fn post_request<S>(
	stream: S,
	request: Request<Full<Bytes>>,
	response_limit: usize,
	deadline: Instant,
) -> Result<Response<String>, PostError>
where
	S: AsyncRead + AsyncWrite + Unpin,
{
	let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
		.await
		.map_err(|e| PostError::NotSent(format!("Handshake: {:?}", e)))?;
	let mut status = None;
	let response = async {
		let response = sender
			.send_request(request)
			.await
			.map_err(|e| PostError::PossiblySent(format!("{:?}", e), None, None))?;
		status = Some(response.status());
		read_response(response, response_limit).await
	};
	timeout_at(deadline, async {
		tokio::pin!(response);
		// Keep the connection scoped to the request
		tokio::select! {
			result = &mut response => result,
			result = connection => {
				if let Err(e) = result {
					error!("Tor connection error: {}", e);
				}
				response.await
			}
		}
	})
	.await
	.map_err(|_| PostError::PossiblySent("Tor request timed out".into(), status, None))?
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
	if config.proxy_config() != &Default::default() {
		warn!("Integrated Tor proxy configuration is ignored");
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
	let mut cached_client_config = ARTI_CLIENT_CONFIG.lock().await;
	let connect_timeout = request_timeout(config);
	builder.stream_timeouts().connect_timeout(connect_timeout);
	let client_config = builder
		.build()
		.map_err(|e| Error::Config(format!("{:?}", e)))?;
	if let Some((client, c, timeout)) = cached_client_config.as_mut() {
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
					"Tor client configuration changed while in use".into(),
				));
			}
			debug!("Tor config changed, rebuild client.");
			*cached_client_config = None;
		}
	}
	let client = launch_client(client_config, config).await?;
	cached_client_config.replace((client.clone(), arti_config.clone(), connect_timeout));
	Ok((client, arti_config))
}

/// Launch tor client from provided configuration.
async fn launch_client<T>(
	client_config: TorClientConfig,
	config: &T,
) -> Result<Arc<TorClient<TokioNativeTlsRuntime>>, Error>
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
		// Arti runtime creation enters block_on internally
		let runtime = tokio::task::spawn_blocking(runtime)
			.await
			.map_err(|e| Error::Generic(format!("{:?}", e)))??;
		let client = TorClient::with_runtime(runtime)
			.config(client_config)
			.create_unbootstrapped_async()
			.await
			.map_err(|e| Error::Generic(format!("{:?}", e)))?;
		client
			.bootstrap()
			.await
			.map_err(|e| Error::Generic(format!("{:?}", e)))?;
		info!("Tor client bootstrapped successfully");
		Ok(client)
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
	use super::{build_post_request, post_request};
	use hyper::{Method, Uri};
	use std::collections::HashMap;
	use std::time::Duration;
	use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
	use tokio::sync::oneshot;
	use tokio::time::{timeout, Instant};

	#[test]
	fn builds_origin_form_request() {
		let url: Uri = "http://example.onion:8080/v1?test=1".parse().unwrap();
		let mut headers = HashMap::new();
		headers.insert("key", "value");
		let request = build_post_request("{}".to_string(), Some(headers), &url).unwrap();

		assert_eq!(request.method(), Method::POST);
		assert_eq!(request.uri(), "/v1?test=1");
		assert_eq!(request.headers()["host"], "example.onion:8080");
		assert_eq!(request.headers()["key"], "value");
	}

	async fn read_headers(stream: &mut DuplexStream) {
		let mut headers = Vec::new();
		while !headers.ends_with(b"\r\n\r\n") {
			headers.push(stream.read_u8().await.unwrap());
		}
	}

	#[tokio::test]
	async fn request_cancel() {
		for response in ["", "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nx"] {
			let (stream, mut peer) = tokio::io::duplex(1024);
			let (sent, received) = oneshot::channel();
			let server = tokio::spawn(async move {
				read_headers(&mut peer).await;
				peer.write_all(response.as_bytes()).await.unwrap();
				sent.send(()).unwrap();
				assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
			});
			let url = "http://example.onion/".parse().unwrap();
			let request = build_post_request(String::new(), None, &url).unwrap();
			let task = tokio::spawn(async move {
				post_request(
					stream,
					request,
					100,
					Instant::now() + Duration::from_secs(1),
				)
				.await
			});
			timeout(Duration::from_secs(1), received)
				.await
				.unwrap()
				.unwrap();
			task.abort();
			assert!(task.await.unwrap_err().is_cancelled());
			timeout(Duration::from_secs(1), server)
				.await
				.unwrap()
				.unwrap();
		}
	}

	#[tokio::test]
	async fn request_complete() {
		let (stream, mut peer) = tokio::io::duplex(1024);
		let server = async move {
			read_headers(&mut peer).await;
			peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
				.await
				.unwrap();
			assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
		};
		let url = "http://example.onion/".parse().unwrap();
		let request = build_post_request(String::new(), None, &url).unwrap();
		let deadline = Instant::now() + Duration::from_secs(1);
		let response = post_request(stream, request, 2, deadline);
		let (_, response) = timeout(Duration::from_secs(1), async {
			tokio::join!(server, response)
		})
		.await
		.unwrap();
		assert_eq!(response.unwrap().body(), "ok");
	}
}
