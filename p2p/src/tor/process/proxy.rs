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

use crate::tor::config::TorProxyConfig;
use crate::tor::process::types::Error;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::str;
use url::Host;

/// Tor Proxy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TorProxy {
	/// proxy type used for the proxy, e.g. "socks4", "socks5", "http", "https"
	pub transport: Option<String>,
	/// Proxy address for the proxy, e.g. IP:PORT or Hostname
	pub address: Option<String>,
	/// Username for the proxy authentification
	pub username: Option<String>,
	/// Password for the proxy authentification
	pub password: Option<String>,
	/// computer goes through a firewall that only allows connections to certain ports
	pub allowed_port: Option<Vec<u16>>,
}

impl TorProxy {
	fn parse_host_port(addr: &str) -> Result<(String, Option<String>), Error> {
		let host: String;
		let str_port: Option<String>;
		let address = addr
			.chars()
			.filter(|c| !c.is_whitespace())
			.collect::<String>();
		if address.starts_with('[') {
			let split = address.split_once("]:").unwrap();
			host = split.0.to_string();
			str_port = Some(split.1.to_string());
		} else if address.contains(":") && !address.ends_with(":") {
			let split = address.split_once(":").unwrap();
			host = split.0.to_string();
			str_port = Some(split.1.to_string());
		} else {
			host = address.to_string();
			str_port = None;
		};
		Ok((host, str_port))
	}

	pub fn parse_address(addr: &str) -> Result<(String, Option<u16>), Error> {
		let (host, str_port) = TorProxy::parse_host_port(addr)?;
		let host = Host::parse(&host)
			.map_err(|_e| Error::Proxy(format!("Invalid host address: {}", host)))?;
		let port = if let Some(p) = str_port {
			let res = p
				.parse::<u16>()
				.map_err(|_e| Error::Proxy(format!("Invalid port number: {}", p)))?;
			Some(res)
		} else {
			None
		};
		Ok((host.to_string(), port))
	}

	pub fn to_hashmap(self) -> Result<HashMap<String, String>, Error> {
		let mut hm = HashMap::new();
		if let Some(ports) = self.allowed_port {
			let mut allowed_ports = "".to_string();
			let last_port = ports.last().unwrap().to_owned();
			for port in ports.clone() {
				allowed_ports.push_str(format!("*:{}", port).as_str());
				if port != last_port {
					allowed_ports.push(',');
				}
			}
			hm.insert("ReachableAddresses".to_string(), allowed_ports);
		}

		let transport = match self.transport {
			Some(t) => t,
			None => return Ok(hm),
		};
		match transport.as_str() {
			"socks4" => {
				hm.insert("Socks4Proxy".to_string(), self.address.unwrap());
				Ok(hm)
			}
			"socks5" => {
				hm.insert("Socks5Proxy".to_string(), self.address.unwrap());

				if let Some(s) = self.username {
					hm.insert("Socks5ProxyUsername".to_string(), s);
				}
				if let Some(s) = self.password {
					hm.insert("Socks5ProxyPassword".to_string(), s);
				}
				Ok(hm)
			}
			"http" | "https" | "http(s)" => {
				hm.insert("HTTPSProxy".to_string(), self.address.unwrap());

				if let Some(user) = self.username {
					let pass = self.password.unwrap_or("".to_string());
					hm.insert(
						"HTTPSProxyAuthenticator".to_string(),
						format!("{}:{}", user, pass),
					);
				}
				Ok(hm)
			}
			_ => Ok(hm),
		}
	}
}

impl TryFrom<TorProxyConfig> for TorProxy {
	type Error = Error;

	fn try_from(tb: TorProxyConfig) -> Result<Self, Self::Error> {
		if let Some(t) = tb.transport {
			let transport = t.to_lowercase();
			match transport.as_str() {
				"socks4" | "socks5" | "http" | "https" | "http(s)" => {
					// Can't parse socket address --> trying to parse a domain name
					if let Some(address) = tb.address {
						let address_addr: String;
						let (host, port) = TorProxy::parse_address(&address)?;
						if let Some(p) = port {
							address_addr = format!("{}:{}", host, p);
						} else {
							address_addr = host
						}
						Ok(TorProxy {
							transport: Some(transport),
							address: Some(address_addr),
							username: tb.username,
							password: tb.password,
							allowed_port: tb.allowed_port,
						})
					} else {
						let msg = format!(
							"Missing proxy address: {} - must be <IP:PORT> or <Hostname>",
							transport
						);
						Err(Error::Proxy(msg))
					}
				}
				// Missing transport type
				_ => {
					let msg = format!(
						"Invalid proxy transport: {} - must be socks4/socks5/http(s)",
						transport
					);
					Err(Error::Proxy(msg))
				}
			}
		} else {
			// In case the user want to allow only some ports
			let ports = tb.allowed_port.unwrap();
			Ok(TorProxy {
				allowed_port: Some(ports),
				..TorProxy::default()
			})
		}
	}
}
