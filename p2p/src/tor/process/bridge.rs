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

use crate::tor::config::TorBridgeConfig;
use crate::tor::process::proxy::TorProxy;

use crate::tor::process::types::Error;
use base64;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::net::SocketAddr;
use std::{env, str};
use url::{Host, Url};

#[cfg(windows)]
const OBFS4_EXE_NAME: &str = "obfs4proxy.exe";
#[cfg(not(windows))]
const OBFS4_EXE_NAME: &str = "obfs4proxy";

#[cfg(windows)]
const SNOWFLAKE_EXE_NAME: &str = "snowflake-client.exe";
#[cfg(not(windows))]
const SNOWFLAKE_EXE_NAME: &str = "snowflake-client";

pub struct FlagParser<'a> {
	/// line left to be parsed
	line: &'a str,
	/// all flags, bool flags and flags that takes a value
	flags: Vec<&'a str>,
	/// bool flags, present in the client line
	bool_flags: Vec<&'a str>,
	/// is current parsed flag is a bool
	is_bool_flag: bool,
	// parsing client or bridge line
	client: bool,
}

/// Helps to retrieve flags and its value whether on the bridge or client option line
impl<'a> FlagParser<'a> {
	pub fn new(line: &'a str, flags: Vec<&'a str>, bool_flags: Vec<&'a str>, client: bool) -> Self {
		Self {
			line,
			flags,
			bool_flags,
			is_bool_flag: false,
			client,
		}
	}

	/// Used only on the client option line parsing, help to retrieve a known flags
	fn flag(&mut self) -> usize {
		let mut split_index = 0;
		let line = self.line.split_whitespace();
		self.is_bool_flag = false;
		for is_flag in line {
			let index = self.flags.iter().position(|&flag| flag == is_flag);
			if let Some(m) = index {
				let i = self.line.find(is_flag).unwrap();
				split_index = i + is_flag.len() + 1;
				let idx_b_flag = self
					.bool_flags
					.iter()
					.position(|&bool_flag| bool_flag == is_flag);
				if let Some(i) = idx_b_flag {
					self.is_bool_flag = true;
					self.bool_flags.remove(i);
				}
				self.flags.remove(m);
				return split_index;
			}
		}
		split_index
	}

	/// Determine at which index we should take the value linked to its flags
	fn end(&mut self, is_bool_flag: bool, right: &str) -> usize {
		if is_bool_flag {
			0
		} else if let Some(stripped) = right.strip_prefix('"') {
			stripped.find('"').unwrap_or(0) + 2
		} else {
			right.find(' ').unwrap_or(right.len())
		}
	}
}

impl<'a> Iterator for FlagParser<'a> {
	type Item = (&'a str, &'a str);

	fn next(&mut self) -> Option<Self::Item> {
		let (left, right) = if self.client {
			// Client parser
			let split_index = self.flag();
			let (l, r) = self.line.split_at(split_index);
			(l, r)
		} else {
			// Bridge parser
			let split_index = self.line.find("=")?;
			let (l, r) = self.line.split_at(split_index + 1);
			(l, r)
		};
		let end = self.end(self.is_bool_flag, right);
		let key = left.split_whitespace().last()?;
		let val = &right[..end];
		self.line = right[end..].trim();
		Some((key, val))
	}
}

/// Every args field that could be in the bridge line
/// obfs4 args : https://github.com/Yawning/obfs4/blob/40245c4a1cf221395c59d1f4bf274127045352f9/transports/obfs4/obfs4.go#L86-L91
/// meek_lite args : https://github.com/Yawning/obfs4/blob/40245c4a1cf221395c59d1f4bf274127045352f9/transports/meeklite/meek.go#L93-L127
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct Transport {
	/// transport type: obfs4, meek_lite, meek, snowflake
	pub transport: Option<String>,
	/// server address
	pub server: Option<String>,
	/// fingerprint
	pub fingerprint: Option<String>,
	/// certificate (obfs4)
	pub cert: Option<String>,
	/// IAT obfuscation: 0 disabled, 1 enabled, 2 paranoid (obfs4)
	pub iatmode: Option<String>,
	/// URL of signaling broker (meek)
	pub url: Option<String>,
	/// optional - front domain (meek)
	pub front: Option<String>,
	/// optional - URL of AMP cache to use as a proxy for signaling (meek)
	pub utls: Option<String>,
	/// optional - HPKP disable argument. (meek)
	pub disablehpkp: Option<String>,
}

impl Transport {
	/// Parse the server address of the bridge line
	fn parse_socketaddr_arg(arg: Option<&&str>) -> Result<String, Error> {
		match arg {
			Some(addr) => {
				let address = addr.parse::<SocketAddr>().map_err(|_e| {
					Error::Bridge(format!("Invalid bridge server address: {}", addr))
				})?;
				Ok(address.to_string())
			}
			None => {
				let msg = "Missing bridge server address".to_string();
				Err(Error::Bridge(msg))
			}
		}
	}

	/// Parse the fingerprint of the bridge line (obfs4/snowflake/meek)
	fn parse_fingerprint_arg(arg: Option<&&str>) -> Result<Option<String>, Error> {
		match arg {
			Some(f) => {
				let fgp = f.to_owned();
				let is_hex = fgp.chars().all(|c| c.is_ascii_hexdigit());
				let fingerprint = fgp.to_uppercase();
				if !(is_hex && fingerprint.len() == 40) {
					let msg = format!("Invalid fingerprint: {}", fingerprint);
					return Err(Error::Bridge(msg));
				}
				Ok(Some(fingerprint))
			}
			None => Ok(None),
		}
	}
	/// Parse the certificate of the bridge line (obfs4)
	pub fn parse_cert_arg(arg: &str) -> Result<String, Error> {
		let cert_vec = base64::decode(arg).map_err(|_e| {
			Error::Bridge(format!(
				"Invalid certificate, error decoding bridge certificate: {}",
				arg
			))
		})?;
		if cert_vec.len() != 52 {
			let msg = format!("Invalid certificate: {}", arg);
			return Err(Error::Bridge(msg));
		}
		Ok(arg.to_string())
	}
	/// Parse the iatmode of the bridge line (obfs4)
	pub fn parse_iatmode_arg(arg: &str) -> Result<String, Error> {
		let iatmode = arg.parse::<u8>().unwrap_or(0);
		if !((0..3).contains(&iatmode)) {
			let msg = format!("Invalid iatmode: {}, must be between 0 and 2", iatmode);
			return Err(Error::Bridge(msg));
		}
		Ok(iatmode.to_string())
	}

	/// Parse the max value for the arg -max in the client line option (snowflake)
	fn parse_hpkp_arg(arg: &str) -> Result<String, Error> {
		let max = arg.parse::<bool>().map_err(|_e| {
			Error::Bridge(format!(
				"Invalid -max value: {}, must be \"true\" or \"false\"",
				arg
			))
		})?;
		Ok(max.to_string())
	}
}

// Client Plugin such as snowflake or obfs4proxy
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct PluginClient {
	// Path plugin client
	pub path: Option<String>,
	// Plugin client option
	pub option: Option<String>,
}

impl PluginClient {
	/// Get the hashmap key(argument) and attached value of the client option line.
	pub fn get_flags(s: &str) -> HashMap<&str, &str> {
		let flags = vec![
			"-url",
			"-front",
			"-ice",
			"-log",
			"-log-to-state-dir",
			"-keep-local-addresses",
			"-unsafe-logging",
			"-max",
			"-loglevel",
			"-enableLogging",
			"-unsafeLogging",
		];
		let bool_flags = vec![
			"-log-to-state-dir",
			"-keep-local-addresses",
			"-unsafe-logging",
			"-enableLogging",
			"-unsafeLogging",
		];
		FlagParser::new(s, flags, bool_flags, true).collect()
	}

	/// Try to find the plugin client path
	pub fn get_client_path(plugin: &str) -> Result<String, Error> {
		let plugin_path = env::var_os("PATH").and_then(|path| {
			env::split_paths(&path)
				.filter_map(|dir| {
					let full_path = dir.join(plugin);
					if full_path.is_file() {
						Some(full_path)
					} else {
						None
					}
				})
				.next()
		});
		match plugin_path {
			Some(path) => Ok(path.into_os_string().into_string().unwrap()),
			None => {
				let msg = format!("Transport client \"{}\" is missing, make sure it's installed and on your path.", plugin);
				Err(Error::Bridge(msg))
			}
		}
	}

	/// Parse the URL value for the arg -url in the client line option (snowflake)
	fn parse_url_arg(arg: &str) -> Result<String, Error> {
		let url = arg
			.parse::<Url>()
			.map_err(|_e| Error::Bridge(format!("Invalid -url value: {}", arg)))?;
		Ok(url.to_string())
	}

	/// Parse the DNS domain value for the arg -front in the client line option (snowflake)
	fn parse_front_arg(arg: &str) -> Result<String, Error> {
		let front = Host::parse(arg)
			.map_err(|_e| Error::Bridge(format!("Invalid -front hostname value: {}", arg)))?;
		match front {
			Host::Domain(_) => Ok(front.to_string()),
			Host::Ipv4(_) | Host::Ipv6(_) => {
				let msg = format!(
					"Invalid front argument: {}, in the client option. Must be a DNS Domain",
					front
				);
				Err(Error::Bridge(msg))
			}
		}
	}

	/// Parse the ICE address value for the arg -ice in the client line option (snowflake)
	fn parse_ice_arg(arg: &str) -> Result<String, Error> {
		let ice_addr = arg.trim();
		let vec_ice_addr = ice_addr.split(",");
		for addr in vec_ice_addr {
			let addr = addr.to_lowercase();
			if addr.starts_with("stun:") || addr.starts_with("turn:") {
				let address = addr.replace("stun:", "").replace("turn:", "");
				let _p_address = TorProxy::parse_address(&address)
					.map_err(|e| Error::Bridge(format!("{:?}", e)))?;
			} else {
				let msg = format!(
					"Invalid ICE address: {}. Must be a stun or turn address",
					addr
				);
				return Err(Error::Bridge(msg));
			}
		}
		Ok(ice_addr.to_string())
	}

	/// Parse the max value for the arg -max in the client line option (snowflake)
	fn parse_max_arg(arg: &str) -> Result<String, Error> {
		match arg.parse::<u16>() {
			Ok(max) => Ok(max.to_string()),
			Err(_e) => {
				let msg = format!("Invalid -max argument: {} in the client option.", arg);
				Err(Error::Bridge(msg))
			}
		}
	}

	/// Parse the loglevel value for the arg -loglevel in the client line option (obfs4)
	fn parse_loglevel_arg(arg: &str) -> Result<String, Error> {
		let log_level = arg.to_uppercase();
		match log_level.as_str() {
			"ERROR" | "WARN" | "INFO" | "DEBUG" => Ok(log_level.to_string()),
			_ => {
				let msg = format!("Invalid log level argurment: {}, in the client option. Must be: ERROR, WARN, INFO or DEBUG", log_level);
				Err(Error::Bridge(msg))
			}
		}
	}

	/// Parse and verify if the client option line of obfs4proxy or snowflake are correct
	/// Obfs4proxy client args : https://github.com/Yawning/obfs4/blob/40245c4a1cf221395c59d1f4bf274127045352f9/obfs4proxy/obfs4proxy.go#L313-L316
	/// Snowflake client args : https://gitlab.torproject.org/tpo/anti-censorship/pluggable-transports/snowflake/-/blob/main/client/snowflake.go#L123-132
	pub fn parse_client(option: &str, snowflake: bool) -> Result<String, Error> {
		let hm_flags = PluginClient::get_flags(option);
		let mut string = String::from("");
		if snowflake {
			let (ck_url, ck_ice) = (hm_flags.contains_key("-url"), hm_flags.contains_key("-ice"));
			if !(ck_url || ck_ice) {
				let msg = if !ck_url {
					"Missing URL argurment for snowflake transport, specify \"-url\"".to_string()
				} else {
					"Missing ICE argurment for snowflake transport, specify \"-ice\"".to_string()
				};
				return Err(Error::Bridge(msg));
			}
			for (key, value) in hm_flags {
				let p_value = match key {
					"-url" => PluginClient::parse_url_arg(value)?,
					"-front" => PluginClient::parse_front_arg(value)?,
					"-ice" => PluginClient::parse_ice_arg(value)?,
					"-ampcache" => value.to_string(),
					"-log" => value.to_string(),
					"-log-to-state-dir" => String::from(""),
					"-keep-local-addresses" => String::from(""),
					"-unsafe-logging" => String::from(""),
					"-max" => PluginClient::parse_max_arg(value)?,
					_ => continue,
				};
				string.push_str(format!(" {} {}", key, p_value).trim_end())
			}
		} else {
			for (key, value) in hm_flags {
				let p_value = match key {
					"-loglevel" => PluginClient::parse_loglevel_arg(value)?,
					"-enableLogging" => String::from(""),
					"-unsafeLogging" => String::from(""),
					_ => continue,
				};
				string.push_str(format!(" {} {}", key, p_value).trim_end())
			}
		}
		let p_string = string.trim_start().to_string();
		Ok(p_string)
	}
}

/// Tor Bridge Field
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct TorBridge {
	/// tor bridge (transport field)
	pub bridge: Transport,
	// tor bridge plugin client (path and option)
	pub client: PluginClient,
}

impl TorBridge {
	/// Get the hashmap key(argument) and attached value of the bridge line. r
	pub fn get_flags(s: &str) -> HashMap<&str, &str> {
		FlagParser::new(s, vec![], vec![], false).collect()
	}

	/// Bridge and client option convertion to hashmap, facility for the writing of the torrc config
	pub fn to_hashmap(&self) -> Result<HashMap<String, String>, Error> {
		let bridge = self.bridge.clone();
		let client = self.client.clone();
		let transport = bridge.transport.as_ref().unwrap().as_str();
		let mut ret_val = HashMap::new();
		match transport {
			"obfs4" => {
				let string_un = &String::from("");
				let chskey = "ClientTransportPlugin".to_string();
				let chsvalue = format!(
					"{} exec {} {}",
					transport,
					client.path.as_ref().unwrap(),
					client.option.as_ref().unwrap_or(string_un)
				);
				ret_val.insert(chskey, chsvalue);

				let hskey = "Bridge".to_string();
				let mut hsvalue = format!("{} {}", transport, bridge.server.as_ref().unwrap());
				if let Some(fingerprint) = bridge.fingerprint {
					hsvalue.push_str(format!(" {}", fingerprint).as_str())
				}
				hsvalue.push_str(format!(" cert={}", bridge.cert.unwrap()).as_str());
				hsvalue.push_str(format!(" iat-mode={}", bridge.iatmode.unwrap()).as_str());
				ret_val.insert(hskey, hsvalue);

				Ok(ret_val)
			}

			"meek_lite" => {
				let chskey = "ClientTransportPlugin".to_string();
				let mut chsvalue = format!("{} exec {}", transport, client.path.as_ref().unwrap());
				if let Some(option) = client.option {
					chsvalue.push_str(format!(" {}", option).as_str())
				}
				ret_val.insert(chskey, chsvalue);

				let hskey = "Bridge".to_string();
				let mut hsvalue = format!("{} {}", transport, bridge.server.as_ref().unwrap());
				if let Some(fingerprint) = bridge.fingerprint {
					hsvalue.push_str(format!(" {}", fingerprint).as_str())
				}

				hsvalue.push_str(format!(" url={}", bridge.url.as_ref().unwrap()).as_str());

				if let Some(front) = bridge.front {
					hsvalue.push_str(format!(" front={}", front).as_str())
				}
				if let Some(utls) = bridge.utls {
					hsvalue.push_str(format!(" utls={}", utls).as_str())
				}
				if let Some(disablehpkp) = bridge.disablehpkp {
					hsvalue.push_str(format!(" disableHPKP={}", disablehpkp).as_str())
				}
				ret_val.insert(hskey, hsvalue);
				Ok(ret_val)
			}

			"snowflake" => {
				let chskey = "ClientTransportPlugin".to_string();
				let chsvalue = format!(
					"{} exec {} {}",
					transport,
					client.path.as_ref().unwrap(),
					client.option.as_ref().unwrap()
				);
				ret_val.insert(chskey, chsvalue);

				let hskey = "Bridge".to_string();
				let mut hsvalue = format!("{} {}", transport, bridge.server.as_ref().unwrap());
				if let Some(fingerprint) = bridge.fingerprint {
					hsvalue.push_str(format!(" {}", fingerprint).as_str())
				}
				ret_val.insert(hskey, hsvalue);
				Ok(ret_val)
			}

			_ => {
				let msg = format!(
					"Invalid transport method: {} - must be obfs4/meek_lite/meek/snowflake",
					transport
				);
				Err(Error::Bridge(msg))
			}
		}
	}
}

impl TryFrom<TorBridgeConfig> for TorBridge {
	type Error = Error;

	fn try_from(tbc: TorBridgeConfig) -> Result<Self, Self::Error> {
		let bridge = match tbc.bridge_line {
			Some(b) => b,
			None => return Ok(TorBridge::default()),
		};
		let flags = TorBridge::get_flags(&bridge);
		let split = bridge.split_whitespace().collect::<Vec<&str>>();
		let mut iter = split.iter();
		let transport = iter.next().unwrap().to_lowercase();
		match transport.as_str() {
			"obfs4" => {
				let socketaddr = Transport::parse_socketaddr_arg(iter.next())?;
				let fingerprint = Transport::parse_fingerprint_arg(iter.next())?;
				let cert = match flags.get_key_value("cert=") {
					Some(hm) => Transport::parse_cert_arg(hm.1)?,
					None => {
						let msg = "Missing cert argurment in obfs4 transport, specify \"cert=\""
							.to_string();
						return Err(Error::Bridge(msg));
					}
				};
				let iatmode = match flags.get_key_value("iat-mode=") {
					Some(hm) => Transport::parse_iatmode_arg(hm.1)?,
					None => String::from("0"),
				};
				let path = PluginClient::get_client_path(OBFS4_EXE_NAME)?;
				let option = match tbc.client_option {
					Some(o) => Some(PluginClient::parse_client(&o, false)?),
					None => None,
				};
				let tbpc = TorBridge {
					bridge: Transport {
						transport: Some("obfs4".into()),
						server: Some(socketaddr.to_string()),
						fingerprint,
						cert: Some(cert),
						iatmode: Some(iatmode),
						..Transport::default()
					},
					client: PluginClient {
						path: Some(path),
						option,
					},
				};
				Ok(tbpc)
			}

			"meek_lite" | "meek" => {
				let socketaddr = Transport::parse_socketaddr_arg(iter.next())?;
				let fingerprint = Transport::parse_fingerprint_arg(iter.next())?;
				let url = match flags.get_key_value("url=") {
					Some(hm) => PluginClient::parse_url_arg(hm.1)?,
					None => {
						let msg = "Missing url argurment in meek_lite transport, specify \"url=\""
							.to_string();
						return Err(Error::Bridge(msg));
					}
				};
				let front = match flags.get_key_value("front=") {
					Some(hm) => Some(PluginClient::parse_front_arg(hm.1)?),
					None => None,
				};
				let utls = flags.get_key_value("utls=").map(|hm| hm.1.to_string());
				let disablehpkp = match flags.get_key_value("disablehpkp=") {
					Some(hm) => Some(Transport::parse_hpkp_arg(hm.1)?),
					None => None,
				};
				let path = PluginClient::get_client_path(OBFS4_EXE_NAME)?;
				let option = match tbc.client_option {
					Some(o) => Some(PluginClient::parse_client(&o, false)?),
					None => None,
				};
				let tbpc = TorBridge {
					bridge: Transport {
						transport: Some("meek_lite".into()),
						server: Some(socketaddr.to_string()),
						fingerprint,
						url: Some(url),
						front,
						utls,
						disablehpkp,
						..Transport::default()
					},
					client: PluginClient {
						path: Some(path),
						option,
					},
				};
				Ok(tbpc)
			}

			"snowflake" => {
				let socketaddr = Transport::parse_socketaddr_arg(iter.next())?;
				let fingerprint = Transport::parse_fingerprint_arg(iter.next())?;
				let path = PluginClient::get_client_path(SNOWFLAKE_EXE_NAME)?;
				let option = match tbc.client_option {
					Some(o) => PluginClient::parse_client(&o, true)?,
					None => {
						let url =
							"-url https://snowflake-broker.torproject.net.global.prod.fastly.net/";
						let front = "-front cdn.sstatic.net";
						let ice = "-ice stun:stun.l.google.com:19302,stun:stun.voip.blackberry.com:3478,stun:stun.altar.com.pl:3478,stun:stun.antisip.com:3478,stun:stun.bluesip.net:3478,stun:stun.dus.net:3478,stun:stun.epygi.com:3478,stun:stun.sonetel.com:3478,stun:stun.sonetel.net:3478,stun:stun.stunprotocol.org:3478,stun:stun.uls.co.za:3478,stun:stun.voipgate.com:3478,stun:stun.voys.nl:3478";
						format!("{} {} {}", url, front, ice)
					}
				};
				let tbpc = TorBridge {
					bridge: Transport {
						transport: Some("snowflake".into()),
						server: Some(socketaddr.to_string()),
						fingerprint,
						..Transport::default()
					},
					client: PluginClient {
						path: Some(path),
						option: Some(option),
					},
				};
				Ok(tbpc)
			}
			_ => {
				let msg = format!(
					"Invalid transport method: {} - must be obfs4/meek_lite/meek/snowflake",
					transport
				);
				Err(Error::Bridge(msg))
			}
		}
	}
}
