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
	pub fn parse_address(addr: &str) -> Result<(String, Option<u16>), Error> {
		let authority = addr
			.parse::<hyper::http::uri::Authority>()
			.map_err(|_| Error::Proxy("Invalid proxy address".into()))?;
		if addr.contains('@') {
			return Err(Error::Proxy(
				"Proxy credentials must be configured separately".into(),
			));
		}
		let host =
			Host::parse(authority.host()).map_err(|_| Error::Proxy("Invalid proxy host".into()))?;
		let port = authority.port_u16();
		if port.is_none() && (addr != authority.host() || matches!(host, Host::Ipv6(_))) {
			return Err(Error::Proxy("Missing or invalid proxy port".into()));
		}
		Ok((host.to_string(), port))
	}

	pub fn to_hashmap(self) -> Result<HashMap<String, String>, Error> {
		if self
			.username
			.iter()
			.chain(self.password.iter())
			.any(|value| value.contains(['\r', '\n', '\0']))
		{
			return Err(Error::Proxy("Invalid proxy credentials".into()));
		}
		let mut hm = HashMap::new();
		if let Some(ports) = self.allowed_port {
			if ports.is_empty() {
				return Err(Error::Proxy("Allowed ports must not be empty".into()));
			}
			let allowed_ports = ports
				.iter()
				.map(|port| format!("*:{}", port))
				.collect::<Vec<_>>()
				.join(",");
			hm.insert("ReachableAddresses".to_string(), allowed_ports);
		}

		let transport = match self.transport {
			Some(t) => t,
			None => return Ok(hm),
		};
		let address = self
			.address
			.ok_or_else(|| Error::Proxy("Missing proxy address".into()))?;
		match transport.as_str() {
			"socks4" => {
				hm.insert("Socks4Proxy".to_string(), address);
				Ok(hm)
			}
			"socks5" => {
				hm.insert("Socks5Proxy".to_string(), address);

				if let Some(s) = self.username {
					hm.insert("Socks5ProxyUsername".to_string(), s);
				}
				if let Some(s) = self.password {
					hm.insert("Socks5ProxyPassword".to_string(), s);
				}
				Ok(hm)
			}
			"http" | "https" | "http(s)" => {
				hm.insert("HTTPSProxy".to_string(), address);

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
			Ok(TorProxy {
				allowed_port: tb.allowed_port,
				..TorProxy::default()
			})
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn proxy_host_port() {
		for (input, host, port) in [
			("127.0.0.1:9050", "127.0.0.1", Some(9050)),
			("localhost:9050", "localhost", Some(9050)),
			("proxy.example", "proxy.example", None),
			("[::1]:9050", "[::1]", Some(9050)),
		] {
			assert_eq!(TorProxy::parse_address(input).unwrap(), (host.into(), port));
			let proxy = TorProxy::try_from(TorProxyConfig {
				transport: Some("socks5".into()),
				address: Some(input.into()),
				..Default::default()
			})
			.unwrap();
			assert_eq!(proxy.to_hashmap().unwrap()["Socks5Proxy"], input);
		}
		assert!(TorProxy::parse_address("[::1]").is_err());
		assert!(TorProxy::parse_address("localhost:65536").is_err());
		assert!(TorProxy::parse_address("user@localhost:9050").is_err());
	}

	#[test]
	fn proxy_torrc() {
		let mut proxy = TorProxy::try_from(TorProxyConfig {
			transport: Some("socks5".into()),
			address: Some("127.0.0.1:9050".into()),
			username: Some("user".into()),
			password: Some("password".into()),
			allowed_port: Some(vec![80, 443, 80]),
		})
		.unwrap();
		let options = proxy.clone().to_hashmap().unwrap();
		assert_eq!(options["ReachableAddresses"], "*:80,*:443,*:80");
		assert_eq!(options["Socks5ProxyUsername"], "user");
		assert_eq!(options["Socks5ProxyPassword"], "password");
		for (username, password) in [
			("user\nSocksPort 0", "password"),
			("user", "password\rSocksPort 0"),
			("user", "password\0"),
		] {
			proxy.username = Some(username.into());
			proxy.password = Some(password.into());
			assert!(matches!(proxy.clone().to_hashmap(), Err(Error::Proxy(_))));
		}
		assert!(TorProxy {
			allowed_port: Some(vec![]),
			..Default::default()
		}
		.to_hashmap()
		.is_err());
		assert!(TorProxy {
			transport: Some("socks5".into()),
			..Default::default()
		}
		.to_hashmap()
		.is_err());
		assert!(TorProxy::try_from(TorProxyConfig::default())
			.unwrap()
			.to_hashmap()
			.unwrap()
			.is_empty());
	}
}
