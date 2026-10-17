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

use std::fmt;
use std::time::Duration;

pub trait TorConfig {
	/// Tor request timeout in seconds.
	const REQUEST_TIMEOUT_SECS: u64 = 60;
	/// Tor bootstrap timeout in seconds.
	const BOOTSTRAP_TIMEOUT_SECS: u64 = 60;

	/// Check if attempt to send over Tor is needed using provided possible argument at priority.
	fn send_tor(&self, skip_arg: Option<bool>) -> bool;

	/// Request timeout.
	fn request_timeout(&self) -> Duration;

	/// Bootstrap timeout.
	fn bootstrap_timeout(&self) -> Duration;

	/// Get bridge configuration.
	fn bridge_config(&self) -> TorBridgeConfig;

	/// Get proxy configuration.
	fn proxy_config(&self) -> TorProxyConfig;
}

/// Tor Bridge Config
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TorBridgeConfig {
	/// Path to bridge binary to use with integrated Tor library
	pub bridge_bin_path: Option<String>,
	/// Bridge Line
	pub bridge_line: Option<String>,
	/// Client Option
	pub client_option: Option<String>,
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
