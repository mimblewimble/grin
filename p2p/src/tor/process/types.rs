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

// BSD 3-Clause License
//
// Copyright (c) 2016, Dhole
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//
// * Redistributions of source code must retain the above copyright notice, this
//   list of conditions and the following disclaimer.
//
// * Redistributions in binary form must reproduce the above copyright notice,
//   this list of conditions and the following disclaimer in the documentation
//   and/or other materials provided with the distribution.
//
// * Neither the name of the copyright holder nor the names of its
//   contributors may be used to endorse or promote products derived from
//   this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
// FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
// DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
// CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
// OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! Tor process control
//! Derived from https://github.com/Dhole/rust-tor-controller.git

use crate::tor::config::{TorBridgeConfig, TorConfig, TorProxyConfig, BOOTSTRAP_TIMEOUT_SECS};
use crate::tor::ov3::OnionV3Error;
use crate::tor::process::config::{output_tor_listener_config, output_tor_sender_config};
use crate::tor::process::proxy::TorProxy;

use fslock_guard::LockFileGuard;
use grin_util::secp::SecretKey;
use regex::Regex;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, MAIN_SEPARATOR};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use sysinfo::{Process, System, SystemExt};

#[cfg(windows)]
const TOR_EXE_NAME: &str = "tor.exe";
#[cfg(not(windows))]
const TOR_EXE_NAME: &str = "tor";

#[derive(Debug)]
pub enum Error {
	Process(String),
	Proxy(String),
	IO(io::Error),
	PID(String),
	Tor(String, Vec<String>),
	InvalidLogLine,
	InvalidBootstrapLine(String),
	Regex(regex::Error),
	ProcessNotStarted,
	Timeout,
	OnionError(OnionV3Error),
}

pub(crate) type SenderIsolation = Arc<Mutex<Option<String>>>;

pub(crate) struct TorProcess {
	torrc_path: Option<String>,
	completion_percent: u8,
	timeout: u64,
	pub working_dir: Option<String>,
	readers: Vec<thread::JoinHandle<io::Result<u64>>>,
	pub process: Option<Child>,
	sys: System,
	dir_lock: Option<LockFileGuard>,
	sender_config: Option<(String, Vec<TorBridgeConfig>, TorProxyConfig)>,
	pub sender_isolation: SenderIsolation,
}

impl Default for TorProcess {
	fn default() -> Self {
		Self {
			torrc_path: None,
			completion_percent: 100,
			timeout: 0,
			working_dir: None,
			readers: Vec::new(),
			process: None,
			sys: System::new(),
			dir_lock: None,
			sender_config: None,
			sender_isolation: Arc::default(),
		}
	}
}

impl TorProcess {
	/// Start Tor process as listener.
	pub fn start_listener<T>(
		sec_key: SecretKey,
		addr: &str,
		config: &T,
	) -> Result<TorProcess, crate::tor::types::Error>
	where
		T: TorConfig,
	{
		use crate::tor::types::Error;
		info!("Starting external Tor Process listener at {}.", addr);

		let mut tor = TorProcess::default();

		let (hm_tor_bridge, hm_tor_proxy) = bridge_proxy(config)?;

		let tor_dir = config
			.data_dir()
			.join("listener")
			.join(crate::tor::service_address(&sec_key)?);
		let tor_dir_str = tor_dir
			.to_str()
			.ok_or(Error::Config(format!("Bad tor dir {}", tor_dir.display())))?;
		tor.lock_dir(tor_dir_str)
			.map_err(|e| Error::Config(format!("{:?}", e)))?;
		output_tor_listener_config(tor_dir_str, addr, &[sec_key], hm_tor_bridge, hm_tor_proxy)
			.map_err(|e| Error::Config(format!("{:?}", e)))?;

		// Start TOR process
		start_tor(tor_dir_str, &mut tor, config)?;
		Ok(tor)
	}

	/// Start Tor process to send requests.
	pub fn start_sender<T>(config: &T) -> Result<TorProcess, crate::tor::types::Error>
	where
		T: TorConfig,
	{
		use crate::tor::types::Error;

		let socks_proxy_addr = SocketAddr::V4(
			config
				.socks_proxy_addr()
				.parse()
				.map_err(|e| Error::Config(format!("{:?}", e)))?,
		);
		let mut tor = TorProcess::default();
		info!(
			"Starting external Tor process to send at {:?}.",
			socks_proxy_addr
		);

		let (hm_tor_bridge, hm_tor_proxy) = bridge_proxy(config)?;

		let mut tor_dir = config.data_dir();
		tor_dir.push("sender");
		let tor_dir_str = tor_dir
			.to_str()
			.ok_or(Error::Config(format!("Bad tor dir {}", tor_dir.display())))?;
		tor.lock_dir(tor_dir_str)
			.map_err(|e| Error::Config(format!("{:?}", e)))?;
		output_tor_sender_config(
			tor_dir_str,
			socks_proxy_addr.to_string().as_str(),
			hm_tor_bridge,
			hm_tor_proxy,
		)
		.map_err(|e| Error::Config(format!("{:?}", e)))?;

		// Start TOR process
		start_tor(tor_dir_str, &mut tor, config)?;
		tor.sender_config = Some((
			config.socks_proxy_addr(),
			config.bridge_config().to_vec(),
			config.proxy_config().clone(),
		));
		Ok(tor)
	}

	fn lock_dir(&mut self, dir: &str) -> Result<(), Error> {
		fs::create_dir_all(dir).map_err(Error::IO)?;
		let lock = LockFileGuard::try_lock(Path::new(dir).join("lock"))
			.map_err(Error::IO)?
			.ok_or_else(|| Error::Process(format!("Tor data directory is in use: {}", dir)))?;
		let pid_file_name = format!("{}{}pid", dir, MAIN_SEPARATOR);
		// A PID file does not establish ownership
		if Path::new(&pid_file_name).exists() {
			let pid = fs::read_to_string(&pid_file_name).map_err(Error::IO)?;
			let pid = pid
				.parse::<i32>()
				.map_err(|err| Error::PID(format!("{:?}", err)))?;
			if self.get_process(pid).is_some() {
				return Err(Error::Process(format!(
					"Tor data directory is in use: {}",
					dir
				)));
			}
		}
		self.dir_lock = Some(lock);
		self.working_dir(dir);
		Ok(())
	}

	fn get_process(&mut self, pid: i32) -> Option<&Process> {
		self.sys.refresh_all();
		self.sys.process((pid as usize).into())
	}

	pub fn torrc_path(&mut self, torrc_path: &str) -> &mut Self {
		self.torrc_path = Some(torrc_path.to_string());
		self
	}

	pub fn completion_percent(&mut self, completion_percent: u8) -> &mut Self {
		self.completion_percent = completion_percent;
		self
	}

	pub fn timeout(&mut self, timeout: u64) -> &mut Self {
		self.timeout = timeout;
		self
	}

	pub fn working_dir(&mut self, dir: &str) -> &mut Self {
		self.working_dir = Some(dir.to_string());
		self
	}

	// The tor process will have its stdout piped, so if the stdout lines are not consumed they
	// will keep accumulating over time, increasing the consumed memory.
	pub fn launch(&mut self) -> Result<&mut Self, Error> {
		self.launch_command(Command::new(TOR_EXE_NAME))
	}

	fn launch_command(&mut self, mut tor: Command) -> Result<&mut Self, Error> {
		tor.args(["__OwningControllerProcess", &std::process::id().to_string()]);
		if let Some(ref dir) = self.working_dir {
			tor.current_dir(dir);
		}
		if let Some(ref torrc_path) = self.torrc_path {
			tor.args(vec!["-f", torrc_path]);
		}
		let tor_process = tor
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.map_err(|err| {
				let msg = format!("TOR executable (`{}`) not found. Please ensure TOR is installed and on the path: {:?}", TOR_EXE_NAME, err);
				Error::Process(msg)
			})?;

		self.process = Some(tor_process);
		let tor_process = self.process.as_mut().unwrap();
		let mut stderr = tor_process.stderr.take().unwrap();
		self.readers.push(thread::spawn(move || {
			io::copy(&mut stderr, &mut io::sink())
		}));

		if let Some(ref d) = self.working_dir {
			// Keep the PID for diagnostics and conflict checks
			let pid_file_name = format!("{}{}pid", d, MAIN_SEPARATOR);
			let mut file = File::create(pid_file_name).map_err(Error::IO)?;
			file.write_all(format!("{}", tor_process.id()).as_bytes())
				.map_err(Error::IO)?;
		}

		let stdout = BufReader::new(tor_process.stdout.take().unwrap());

		let completion_percent = self.completion_percent;

		let (stdout_tx, stdout_rx) = channel();
		self.readers.push(thread::spawn(move || {
			let mut stdout = stdout;
			let result = Self::parse_tor_stdout(&mut stdout, completion_percent);
			let ready = result.is_ok();
			stdout_tx.send(result).unwrap_or(());
			if ready {
				io::copy(&mut stdout, &mut io::sink())
			} else {
				Ok(0)
			}
		}));
		let result = match stdout_rx.recv_timeout(Duration::from_secs(self.timeout)) {
			Ok(result) => result,
			Err(RecvTimeoutError::Timeout) => Err(Error::Timeout),
			Err(RecvTimeoutError::Disconnected) => {
				Err(Error::Process("Tor stdout reader stopped".into()))
			}
		};
		match result {
			Ok(()) => Ok(self),
			Err(err) => {
				let exit = self.exit_status()?;
				self.kill()?;
				match exit {
					Some(status) => Err(Error::Process(format!(
						"Tor exited during bootstrap: {} ({:?})",
						status, err
					))),
					None => Err(err),
				}
			}
		}
	}

	fn parse_tor_stdout(
		stdout: &mut BufReader<ChildStdout>,
		completion_perc: u8,
	) -> Result<(), Error> {
		let re_bootstrap =
			Regex::new(r"^\[notice] Bootstrapped (?P<perc>[0-9]+)%(.*): ").map_err(Error::Regex)?;

		let timestamp_len = "May 16 02:50:08.792".len();
		let mut warnings = Vec::new();
		let mut raw_line = String::new();

		while stdout
			.read_line(&mut raw_line)
			.map_err(|err| Error::Process(format!("{}", err)))?
			> 0
		{
			{
				if raw_line.len() < timestamp_len + 1 {
					return Err(Error::InvalidLogLine);
				}
				let timestamp = &raw_line[..timestamp_len];
				let line = &raw_line[timestamp_len + 1..raw_line.len() - 1];
				debug!("{} {}", timestamp, line);
				match line.split(' ').nth(0) {
					Some("[notice]") => {
						if let Some("Bootstrapped") = line.split(' ').nth(1) {
							let perc = re_bootstrap
								.captures(line)
								.and_then(|c| c.name("perc"))
								.and_then(|pc| pc.as_str().parse::<u8>().ok())
								.ok_or_else(|| Error::InvalidBootstrapLine(line.to_string()))?;

							if perc >= completion_perc {
								return Ok(());
							}
						}
					}
					Some("[warn]") => warnings.push(line.to_string()),
					Some("[err]") => return Err(Error::Tor(line.to_string(), warnings)),
					_ => (),
				}
			}
			raw_line.clear();
		}
		Err(Error::Process(
			"Tor exited before bootstrap completed".into(),
		))
	}

	pub fn exit_status(&mut self) -> Result<Option<ExitStatus>, Error> {
		self.process
			.as_mut()
			.ok_or(Error::ProcessNotStarted)?
			.try_wait()
			.map_err(Error::IO)
	}

	pub fn matches_sender<T: TorConfig>(&self, config: &T) -> bool {
		self.working_dir.as_deref() == config.data_dir().join("sender").to_str()
			&& self
				.sender_config
				.as_ref()
				.map(|(socks, bridge, proxy)| {
					socks == &config.socks_proxy_addr()
						&& bridge == config.bridge_config()
						&& proxy == config.proxy_config()
				})
				.unwrap_or(false)
	}

	pub fn kill(&mut self) -> Result<(), Error> {
		if let Some(process) = self.process.as_mut() {
			if process.try_wait().map_err(Error::IO)?.is_none() {
				if let Err(error) = process.kill() {
					if process.try_wait().map_err(Error::IO)?.is_none() {
						return Err(Error::IO(error));
					}
				}
			}
			process.wait().map_err(Error::IO)?;
		}
		let mut result = Ok(());
		for reader in self.readers.drain(..) {
			let joined = reader
				.join()
				.map_err(|_| Error::Process("Tor reader panicked".into()))
				.and_then(|result| result.map(|_| ()).map_err(Error::IO));
			if result.is_ok() {
				result = joined;
			}
		}
		self.process = None;
		self.dir_lock = None;
		result
	}
}

type BridgeProxyResult =
	Result<(Vec<(String, String)>, HashMap<String, String>), crate::tor::types::Error>;

fn bridge_options(bridges: &[TorBridgeConfig]) -> Result<Vec<(String, String)>, crate::tor::Error> {
	let mut hm_tor_bridge = Vec::new();
	let mut transports = HashMap::new();
	for config in bridges {
		let bridge = config.parse()?;
		if let Some(protocol) = bridge.get_transport().filter(|p| !p.is_empty()) {
			let args = config.arguments(protocol)?;
			let path = config.binary(protocol)?;
			if std::iter::once(&path)
				.chain(args.iter())
				.any(|arg| arg.is_empty() || arg.chars().any(char::is_whitespace))
			{
				return Err(crate::tor::Error::Config(
					"External Tor bridge arguments cannot be empty or contain whitespace".into(),
				));
			}
			let command = format!("{} exec {} {}", protocol, path, args.join(" "));
			if let Some(previous) = transports.insert(protocol.to_string(), command.clone()) {
				if previous != command {
					return Err(crate::tor::Error::Config(
						"Conflicting bridge binaries or options".into(),
					));
				}
			} else {
				hm_tor_bridge.push(("ClientTransportPlugin".into(), command));
			}
		}
		hm_tor_bridge.push((
			"Bridge".into(),
			super::bridge::bridge_line(config.bridge_line.as_deref().unwrap_or_default())?,
		));
	}

	Ok(hm_tor_bridge)
}

fn bridge_proxy<T>(config: &T) -> BridgeProxyResult
where
	T: TorConfig,
{
	let hm_tor_bridge = bridge_options(config.bridge_config())?;

	let mut hm_tor_proxy: HashMap<String, String> = HashMap::new();
	let proxy_config = config.proxy_config();
	if proxy_config.transport.is_some() || proxy_config.allowed_port.is_some() {
		let proxy = TorProxy::try_from(proxy_config.clone())
			.map_err(|e| crate::tor::Error::Config(format!("{:?}", e)))?;
		hm_tor_proxy = proxy
			.to_hashmap()
			.map_err(|e| crate::tor::Error::Config(format!("{:?}", e)))?;
	}
	Ok((hm_tor_bridge, hm_tor_proxy))
}

fn start_tor<T>(tor_dir: &str, tor: &mut TorProcess, config: &T) -> Result<(), crate::tor::Error>
where
	T: TorConfig,
{
	let rc_path = fs::canonicalize(tor_dir)
		.map_err(|e| crate::tor::Error::Config(format!("{:?}", e)))?
		.join("torrc");
	let rc_path = rc_path
		.to_str()
		.ok_or_else(|| crate::tor::Error::Config(format!("Bad tor dir {}", rc_path.display())))?;
	tor.torrc_path(rc_path)
		.timeout(
			config
				.bootstrap_timeout_secs()
				.filter(|s| *s > 0)
				.unwrap_or(BOOTSTRAP_TIMEOUT_SECS),
		)
		.completion_percent(100)
		.launch()
		.map_err(|e| crate::tor::Error::Generic(format!("{:?}", e)))?;
	Ok(())
}

impl Drop for TorProcess {
	// kill the child
	fn drop(&mut self) {
		debug!("Dropping TOR process");
		if let Err(error) = self.kill() {
			error!("Error stopping Tor process: {:?}", error);
		}
	}
}

#[cfg(all(test, unix))]
mod tests {
	use super::*;

	#[test]
	fn bridge_list() {
		let bridge = TorBridgeConfig {
			bridge_line: Some("webtunnel 192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567 url=https://example.com/bridge".into()),
			bridge_bin_path: Some("/tmp/bridge-client".into()),
			client_args: vec!["-enableLogging".into()],
		};
		let options = bridge_options(&[bridge.clone(), bridge.clone()]).unwrap();
		let dir = tempfile::tempdir().unwrap();
		output_tor_sender_config(
			dir.path().to_str().unwrap(),
			"127.0.0.1:9050",
			options,
			HashMap::new(),
		)
		.unwrap();
		let torrc = fs::read_to_string(dir.path().join("torrc")).unwrap();
		assert!(torrc.lines().any(|line| line == "UseBridges 1"));
		assert_eq!(
			torrc
				.lines()
				.filter(|line| *line == format!("Bridge {}", bridge.bridge_line.as_ref().unwrap()))
				.count(),
			2
		);

		assert_eq!(
			torrc
				.lines()
				.filter(|line| line.starts_with("ClientTransportPlugin "))
				.collect::<Vec<_>>(),
			vec!["ClientTransportPlugin webtunnel exec /tmp/bridge-client -enableLogging"]
		);

		let mut changed = bridge.clone();
		changed.bridge_bin_path = Some("/tmp/other".into());
		assert!(bridge_options(&[bridge.clone(), changed]).is_err());
		let mut invalid = bridge.clone();
		invalid.bridge_bin_path = Some("/tmp/bridge client".into());
		assert!(bridge_options(&[invalid]).is_err());
		let mut invalid = bridge;
		invalid.client_args.push("two words".into());
		assert!(bridge_options(&[invalid.clone()]).is_err());
		invalid.client_args = vec![String::new()];
		assert!(bridge_options(&[invalid]).is_err());
	}

	fn command(script: &str) -> Command {
		let mut command = Command::new("/bin/sh");
		command.args(["-c", script]);
		command
	}

	#[test]
	fn bootstrap_exit() {
		let mut process = TorProcess::default();
		process.timeout(2);
		assert!(process.launch_command(command("exit 42")).is_err());
		assert!(process.process.is_none());
		assert!(process.readers.is_empty());
	}

	#[test]
	fn stop_wait() {
		let mut process = TorProcess::default();
		process.timeout(2);
		process
			.launch_command(command(
				r#"set -e
test "$0" = __OwningControllerProcess
test "$1" = "$PPID"
echo 'May 16 02:50:08.792 [notice] Bootstrapped 100% (done): Done'
exec /bin/sleep 60"#,
			))
			.unwrap();
		assert!(process.exit_status().unwrap().is_none());
		process.kill().unwrap();
		assert!(process.process.is_none());
		assert!(process.readers.is_empty());
		process.kill().unwrap();
	}

	#[test]
	fn bootstrap_timeout() {
		let mut process = TorProcess::default();
		process.timeout(1);
		assert!(matches!(
			process.launch_command(command("exec /bin/sleep 60")),
			Err(Error::Timeout)
		));
		assert!(process.process.is_none());
		assert!(process.readers.is_empty());
	}

	#[test]
	fn foreign_pid() {
		let dir = tempfile::tempdir().unwrap();
		let mut owner = TorProcess::default();
		owner.process = Some(command("exec /bin/sleep 60").spawn().unwrap());
		fs::write(
			dir.path().join("pid"),
			owner.process.as_ref().unwrap().id().to_string(),
		)
		.unwrap();
		let mut other = TorProcess::default();
		assert!(other.lock_dir(dir.path().to_str().unwrap()).is_err());
		assert!(owner.exit_status().unwrap().is_none());
		owner.kill().unwrap();
		other.lock_dir(dir.path().to_str().unwrap()).unwrap();
		assert!(owner.lock_dir(dir.path().to_str().unwrap()).is_err());
		other.kill().unwrap();
		owner.lock_dir(dir.path().to_str().unwrap()).unwrap();
	}
}
