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

use crate::tor::process::proxy::TorProxy;
use crate::tor::Error;
use std::ffi::OsStr;
use url::{Host, Url};

pub(crate) fn binary(transport: &str) -> Result<String, Error> {
	let name = match transport {
		"obfs4" | "meek" | "meek_lite" => "obfs4proxy",
		"snowflake" => "snowflake-client",
		_ => transport,
	};
	#[cfg(windows)]
	let name = &format!("{}.exe", name);
	find_binary(name, &std::env::var_os("PATH").unwrap_or_default())
}

fn find_binary(name: &str, paths: &OsStr) -> Result<String, Error> {
	let path = std::env::split_paths(paths)
		.map(|dir| dir.join(name))
		.find(|path| path.is_file())
		.ok_or_else(|| {
			Error::Config(format!(
				"Transport client \"{}\" is missing from PATH",
				name
			))
		})?;
	path.into_os_string()
		.into_string()
		.map_err(|_| Error::Config("Transport client path is not valid UTF-8".into()))
}

pub(crate) fn bridge_line(line: &str) -> Result<String, Error> {
	let mut line = line.trim();
	let first = line.split_whitespace().next().unwrap_or("");
	if first.eq_ignore_ascii_case("bridge") {
		line = line[first.len()..].trim_start();
	}
	let first = line.split_whitespace().next().unwrap_or("");
	let transport = first.to_ascii_lowercase();
	let transport = if transport == "meek" {
		"meek_lite"
	} else {
		&transport
	};
	let option = |key| {
		line.split_whitespace().find_map(|part| {
			part.split_once('=')
				.filter(|(name, _)| *name == key)
				.map(|(_, value)| value)
		})
	};
	let mut result = line.to_owned();
	match transport {
		"obfs4" => {
			parse_cert_arg(
				option("cert").ok_or_else(|| Error::Config("Missing obfs4 certificate".into()))?,
			)?;
			if let Some(mode) = option("iat-mode") {
				parse_iatmode_arg(mode)?;
			} else {
				result.push_str(" iat-mode=0");
			}
		}
		"meek_lite" => {
			parse_url_arg(option("url").ok_or_else(|| Error::Config("Missing meek URL".into()))?)?;
			if let Some(front) = option("front") {
				parse_front_arg(front)?;
			}
			if let Some(value) = option("disablehpkp") {
				parse_hpkp_arg(value)?;
			}
		}
		"snowflake" => {}
		_ => return Ok(result),
	}
	result.replace_range(..first.len(), transport);
	Ok(result)
}

pub(crate) fn client_args(transport: &str, configured: &[String]) -> Result<Vec<String>, Error> {
	let mut args = if transport == "snowflake" && configured.is_empty() {
		vec![
			"-url".into(), "https://snowflake-broker.torproject.net.global.prod.fastly.net/".into(),
			"-front".into(), "cdn.sstatic.net".into(),
			"-ice".into(), "stun:stun.l.google.com:19302,stun:stun.voip.blackberry.com:3478,stun:stun.altar.com.pl:3478,stun:stun.antisip.com:3478,stun:stun.bluesip.net:3478,stun:stun.dus.net:3478,stun:stun.epygi.com:3478,stun:stun.sonetel.com:3478,stun:stun.sonetel.net:3478,stun:stun.stunprotocol.org:3478,stun:stun.uls.co.za:3478,stun:stun.voipgate.com:3478,stun:stun.voys.nl:3478".into(),
		]
	} else {
		configured.to_vec()
	};
	let mut index = 0;
	while index < args.len() {
		let flag = args[index].as_str();
		let known = match transport {
			"snowflake" => matches!(
				flag,
				"-url" | "-front" | "-ice" | "-max" | "-log" | "-ampcache"
			),
			"obfs4" | "meek" | "meek_lite" => flag == "-loglevel",
			_ => false,
		};
		if !known {
			index += 1;
			continue;
		}
		let value = args
			.get(index + 1)
			.filter(|value| !value.is_empty() && !value.starts_with('-'))
			.ok_or_else(|| Error::Config(format!("Missing or invalid value for {}", flag)))?;
		args[index + 1] = match flag {
			"-url" => parse_url_arg(value)?,
			"-front" => parse_front_arg(value)?,
			"-ice" => parse_ice_arg(value)?,
			"-max" => parse_max_arg(value)?,
			"-loglevel" => parse_loglevel_arg(value)?,
			_ => value.clone(),
		};
		index += 2;
	}
	if transport == "snowflake" && !args.iter().any(|arg| arg == "-url" || arg == "-ice") {
		return Err(Error::Config(
			"Missing snowflake URL or ICE argument".into(),
		));
	}
	Ok(args)
}

/// Parse the certificate of the bridge line (obfs4)
fn parse_cert_arg(arg: &str) -> Result<String, Error> {
	let cert_vec = base64::decode(arg).map_err(|_e| {
		Error::Config(format!(
			"Invalid certificate, error decoding bridge certificate: {}",
			arg
		))
	})?;
	if cert_vec.len() != 52 {
		let msg = format!("Invalid certificate: {}", arg);
		return Err(Error::Config(msg));
	}
	Ok(arg.to_string())
}

/// Parse the iatmode of the bridge line (obfs4)
fn parse_iatmode_arg(arg: &str) -> Result<String, Error> {
	let iatmode = arg
		.parse::<u8>()
		.map_err(|_| Error::Config("Invalid iat-mode".into()))?;
	if !((0..3).contains(&iatmode)) {
		let msg = format!("Invalid iatmode: {}, must be between 0 and 2", iatmode);
		return Err(Error::Config(msg));
	}
	Ok(iatmode.to_string())
}

/// Parse the disablehpkp value of the bridge line (meek)
fn parse_hpkp_arg(arg: &str) -> Result<String, Error> {
	let value = arg.parse::<bool>().map_err(|_e| {
		Error::Config(format!(
			"Invalid disablehpkp value: {}, must be \"true\" or \"false\"",
			arg
		))
	})?;
	Ok(value.to_string())
}

/// Parse the URL value for the arg -url in the client line option (snowflake)
fn parse_url_arg(arg: &str) -> Result<String, Error> {
	let url = arg
		.parse::<Url>()
		.map_err(|_e| Error::Config(format!("Invalid -url value: {}", arg)))?;
	Ok(url.to_string())
}

/// Parse the DNS domain value for the arg -front in the client line option (snowflake)
fn parse_front_arg(arg: &str) -> Result<String, Error> {
	let front = Host::parse(arg)
		.map_err(|_e| Error::Config(format!("Invalid -front hostname value: {}", arg)))?;
	match front {
		Host::Domain(_) => Ok(front.to_string()),
		Host::Ipv4(_) | Host::Ipv6(_) => {
			let msg = format!(
				"Invalid front argument: {}, in the client option. Must be a DNS Domain",
				front
			);
			Err(Error::Config(msg))
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
			let _p_address =
				TorProxy::parse_address(&address).map_err(|e| Error::Config(format!("{:?}", e)))?;
		} else {
			let msg = format!(
				"Invalid ICE address: {}. Must be a stun or turn address",
				addr
			);
			return Err(Error::Config(msg));
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
			Err(Error::Config(msg))
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
			Err(Error::Config(msg))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::tor::config::TorBridgeConfig;

	#[test]
	fn client_path() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("bridge-client");
		std::fs::write(&file, "").unwrap();
		assert_eq!(
			find_binary("bridge-client", dir.path().as_os_str()).unwrap(),
			file.to_str().unwrap()
		);
		assert!(find_binary("missing", dir.path().as_os_str()).is_err());
		let config = TorBridgeConfig {
			bridge_bin_path: Some(file.to_str().unwrap().into()),
			..Default::default()
		};
		assert_eq!(config.binary("webtunnel").unwrap(), file.to_str().unwrap());
	}

	#[test]
	fn obfs4_options() {
		let line = format!(
			"Bridge obfs4 192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567 cert={}",
			base64::encode([0u8; 52])
		);
		let mut config = TorBridgeConfig {
			bridge_line: Some(line.clone()),
			..Default::default()
		};
		assert!(config
			.parse()
			.unwrap()
			.build()
			.unwrap()
			.to_string()
			.contains("iat-mode=0"));
		config.bridge_line = Some(format!("{} iat-mode=3", line));
		assert!(config.parse().is_err());
		config.bridge_line = Some("obfs4 192.0.2.1:443 cert=bad".into());
		assert!(config.parse().is_err());
	}

	#[test]
	fn meek_options() {
		let line = "meek 192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567";
		assert!(bridge_line(line).is_err());
		let valid = format!(
			"{} url=https://example.com/ front=example.com disablehpkp=true",
			line
		);
		let config = TorBridgeConfig {
			bridge_line: Some(valid.clone()),
			..Default::default()
		};
		assert_eq!(config.parse().unwrap().get_transport(), Some("meek_lite"));
		assert!(bridge_line(&valid.replace("front=example.com", "front=127.0.0.1")).is_err());
		assert!(bridge_line(&valid.replace("disablehpkp=true", "disablehpkp=bad")).is_err());
	}

	#[test]
	fn client_flags() {
		assert!(client_args("snowflake", &["-log-to-state-dir".into()]).is_err());
		let defaults = client_args("snowflake", &[]).unwrap();
		assert_eq!(
			&defaults[..4],
			[
				"-url",
				"https://snowflake-broker.torproject.net.global.prod.fastly.net/",
				"-front",
				"cdn.sstatic.net"
			]
		);
		assert_eq!(defaults[4], "-ice");
		assert!(defaults[5].starts_with("stun:stun.l.google.com:19302,"));
		let mut args = vec![
			"-url".into(),
			"https://example.com/".into(),
			"-log".into(),
			"a b.log".into(),
			"-log-to-state-dir".into(),
		];
		assert_eq!(client_args("snowflake", &args).unwrap(), args);
		args.pop();
		args.pop();
		assert!(client_args("snowflake", &args).is_err());
		assert!(client_args("snowflake", &["-ice".into(), "invalid".into()]).is_err());
		assert!(client_args(
			"snowflake",
			&[
				"-url".into(),
				"https://example.com/".into(),
				"-max".into(),
				"65536".into()
			]
		)
		.is_err());
		assert_eq!(
			client_args(
				"obfs4",
				&["-loglevel".into(), "debug".into(), "-enableLogging".into()]
			)
			.unwrap(),
			["-loglevel", "DEBUG", "-enableLogging"]
		);
		assert!(client_args("obfs4", &["-loglevel".into(), "bad".into()]).is_err());
	}
}
