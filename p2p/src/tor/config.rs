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

use crate::tor::process::bridge;
use crate::tor::Error;

use arti_client::config::BridgeConfigBuilder;
use hyper::Uri;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// Tor request timeout in seconds.
pub const REQUEST_TIMEOUT_SECS: u64 = 60;
/// Tor bootstrap timeout in seconds.
pub const BOOTSTRAP_TIMEOUT_SECS: u64 = 60;
/// Default minimum Tor circuit build timeout in milliseconds.
pub const MIN_CIRCUIT_TIMEOUT_MS: i32 = 60_000;

pub trait TorConfig {
	/// Whether to use integrated Tor library.
	fn use_integrated(&self) -> bool;

	/// Custom request timeout.
	fn request_timeout_secs(&self) -> Option<u64>;

	/// Custom bootstrap timeout.
	fn bootstrap_timeout_secs(&self) -> Option<u64>;

	/// Custom minimum Tor circuit build timeout in milliseconds.
	fn min_circuit_timeout_ms(&self) -> Option<i32>;

	/// Bridge configurations
	fn bridge_config(&self) -> &[TorBridgeConfig];

	/// Upstream proxy for Tor
	fn proxy_config(&self) -> &TorProxyConfig;

	/// Existing proxy for outgoing requests, without starting a Tor sender
	fn outbound_proxy(&self) -> Option<TorOutboundProxy> {
		None
	}

	/// Tor data directory.
	fn data_dir(&self) -> PathBuf;

	/// The address of the socks proxy for external process.
	fn socks_proxy_addr(&self) -> String;
}

pub(crate) fn request_timeout<T: TorConfig>(config: &T) -> Duration {
	Duration::from_secs(
		config
			.request_timeout_secs()
			.filter(|s| *s > 0)
			.unwrap_or(REQUEST_TIMEOUT_SECS),
	)
}

/// Tor Bridge Config
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct TorBridgeConfig {
	/// Path to the bridge binary
	pub bridge_bin_path: Option<String>,
	/// Bridge Line
	pub bridge_line: Option<String>,
	/// Bridge client arguments
	#[serde(default)]
	pub client_args: Vec<String>,
}

impl TorBridgeConfig {
	pub(crate) fn parse(&self) -> Result<BridgeConfigBuilder, Error> {
		let line = self
			.bridge_line
			.as_ref()
			.ok_or_else(|| Error::Config("Missing bridge line".into()))?;
		if line.contains(['\r', '\n']) {
			return Err(Error::Config("Bad bridge line".into()));
		}
		if self
			.bridge_bin_path
			.iter()
			.chain(self.client_args.iter())
			.any(|arg| arg.contains(['\r', '\n', '\0']))
		{
			return Err(Error::Config("Bad bridge client argument".into()));
		}
		let bridge: BridgeConfigBuilder = bridge::bridge_line(line)?
			.parse()
			.map_err(|e| Error::Config(format!("{:?}", e)))?;
		if bridge.get_transport().is_none_or(str::is_empty)
			&& (self.bridge_bin_path.is_some() || !self.client_args.is_empty())
		{
			return Err(Error::Config(
				"Bridge client options require a transport".into(),
			));
		}
		Ok(bridge)
	}

	pub(crate) fn binary(&self, transport: &str) -> Result<String, Error> {
		match &self.bridge_bin_path {
			Some(path) => Ok(path.clone()),
			None => bridge::binary(transport),
		}
	}

	pub(crate) fn arguments(&self, transport: &str) -> Result<Vec<String>, Error> {
		bridge::client_args(transport, &self.client_args)
	}
}

impl fmt::Display for TorBridgeConfig {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{:?}", self)
	}
}

/// Tor Proxy configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TorProxyConfig {
	/// socks4 |socks5 | http(s)
	pub transport: Option<String>,
	/// ip or dns
	pub address: Option<String>,
	/// user for auth - socks5|https(s)
	pub username: Option<String>,
	/// pass for auth - socks5|https(s)
	pub password: Option<String>,
	/// allowed port - proxy
	pub allowed_port: Option<Vec<u16>>,
}

impl fmt::Display for TorProxyConfig {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{:?}", self)
	}
}

/// Existing Tor gateway, separate from Tor's upstream proxy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TorOutboundProxy {
	Socks5(String),
	Http(String),
}

impl TorOutboundProxy {
	pub(crate) fn uri(&self) -> Result<Uri, Error> {
		let (address, schemes) = match self {
			Self::Socks5(address) => (address, &["socks5"][..]),
			Self::Http(address) => (address, &["http", "https"][..]),
		};
		let uri: Uri = address
			.parse()
			.map_err(|_| Error::Config("Bad outbound proxy URL".into()))?;
		if !schemes.contains(&uri.scheme_str().unwrap_or(""))
			|| uri.host().is_none()
			|| uri.authority().is_some_and(|a| a.as_str().contains('@'))
			|| uri
				.authority()
				.is_some_and(|a| a.as_str() != a.host() && a.port_u16().is_none())
			|| uri.path_and_query().is_some_and(|p| p.as_str() != "/")
		{
			return Err(Error::Config("Bad outbound proxy URL".into()));
		}
		Ok(uri)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn proxy_url() {
		assert!(TorOutboundProxy::Http("http://localhost:65536".into())
			.uri()
			.is_err());
		assert!(TorOutboundProxy::Http("http://[::1]".into()).uri().is_ok());
		assert!(TorOutboundProxy::Socks5("socks5://127.0.0.1:9050".into())
			.uri()
			.is_ok());
		assert!(TorOutboundProxy::Socks5("http://127.0.0.1:9050".into())
			.uri()
			.is_err());
		assert!(
			TorOutboundProxy::Http("http://user:pass@localhost:8080".into())
				.uri()
				.is_err()
		);
	}

	#[test]
	fn bridge_fields() {
		let json = r#"{"client_option":"-enableLogging"}"#;
		let error = serde_json::from_str::<TorBridgeConfig>(json).unwrap_err();
		assert!(error.to_string().contains("unknown field `client_option`"));
		let bridge = TorBridgeConfig {
			client_args: vec!["-enableLogging".into()],
			..Default::default()
		};
		let json = serde_json::to_string(&bridge).unwrap();
		assert_eq!(
			serde_json::from_str::<TorBridgeConfig>(&json).unwrap(),
			bridge
		);
	}

	#[test]
	fn bridge_transport() {
		assert!(TorBridgeConfig::default().parse().is_err());
		let mut bridge: TorBridgeConfig = serde_json::from_str(
			r#"{"bridge_line":"192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567"}"#,
		)
		.unwrap();
		assert!(bridge.parse().is_ok());
		bridge.client_args.push("--test".into());
		assert!(bridge.parse().is_err());
	}
}
