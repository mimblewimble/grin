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
//! Derived from from from https://github.com/Dhole/rust-tor-controller.git

extern crate chrono;
extern crate regex;
extern crate timer;

use crate::tor::ov3::OnionV3Error;

use crate::tor::config::{TorConfig, BOOTSTRAP_TIMEOUT_SECS};
use crate::tor::process::bridge::TorBridge;
use crate::tor::process::config::{output_tor_listener_config, output_tor_sender_config};
use crate::tor::process::proxy::TorProxy;
use grin_util::secp::SecretKey;
use regex::Regex;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::{Path, MAIN_SEPARATOR};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;
use std::{cmp, io};
use sysinfo::{Process, ProcessExt, System, SystemExt};

#[cfg(windows)]
const TOR_EXE_NAME: &str = "tor.exe";
#[cfg(not(windows))]
const TOR_EXE_NAME: &str = "tor";

#[derive(Debug)]
pub enum Error {
	Process(String),
	Bridge(String),
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

pub struct TorProcess {
	tor_cmd: String,
	args: Vec<String>,
	torrc_path: Option<String>,
	completion_percent: u8,
	timeout: u64,
	working_dir: Option<String>,
	pub stdout: Option<BufReader<ChildStdout>>,
	pub process: Option<Child>,
	sys: System,
}

impl Default for TorProcess {
	fn default() -> Self {
		Self::new()
	}
}

impl TorProcess {
	pub fn new() -> Self {
		TorProcess {
			tor_cmd: TOR_EXE_NAME.to_string(),
			args: vec![],
			torrc_path: None,
			completion_percent: 100,
			timeout: 0,
			working_dir: None,
			stdout: None,
			process: None,
			sys: System::new(),
		}
	}

	/// Start Tor process as listener.
	pub fn start_listener<T>(
		sec_key: SecretKey,
		tor_dir: String,
		addr: &str,
		config: T,
	) -> Result<TorProcess, crate::tor::types::Error>
	where
		T: TorConfig,
	{
		use crate::tor::types::Error;
		info!("Starting external Tor Process listener at {}.", addr);

		let mut process = TorProcess::new();
		let tor_timeout = config
			.bootstrap_timeout()
			.unwrap_or(Duration::from_secs(BOOTSTRAP_TIMEOUT_SECS))
			.as_secs();

		let mut hm_tor_bridge: HashMap<String, String> = HashMap::new();
		let bridge_config = config.bridge_config();
		if bridge_config.bridge_line.is_some() {
			let bridge_config = TorBridge::try_from(bridge_config)
				.map_err(|e| Error::Config(format!("{:?}", e)))?;
			hm_tor_bridge = bridge_config
				.to_hashmap()
				.map_err(|e| Error::Config(format!("{:?}", e)))?;
		}

		let mut hm_tor_poxy: HashMap<String, String> = HashMap::new();
		let proxy_config = config.proxy_config();
		if proxy_config.transport.is_some() || proxy_config.allowed_port.is_some() {
			let proxy_config =
				TorProxy::try_from(proxy_config).map_err(|e| Error::Config(format!("{:?}", e)))?;
			hm_tor_poxy = proxy_config
				.to_hashmap()
				.map_err(|e| Error::Config(format!("{:?}", e)))?;
		}

		output_tor_listener_config(&tor_dir, addr, &[sec_key], hm_tor_bridge, hm_tor_poxy)
			.map_err(|e| Error::Config(format!("{:?}", e)))?;
		// Start TOR process
		process
			.torrc_path(&format!("{}/torrc", tor_dir))
			.working_dir(&tor_dir)
			.timeout(tor_timeout)
			.completion_percent(100)
			.launch()
			.map_err(|e| Error::Generic(format!("{:?}", e)))?;
		Ok(process)
	}

	/// Start Tor process to send requests.
	pub fn start_sender<T>(
		config: &T,
		socks_proxy_addr: String,
		tor_dir: &Path,
	) -> Result<TorProcess, crate::tor::types::Error>
	where
		T: TorConfig,
	{
		use crate::tor::types::Error;

		let mut tor = TorProcess::new();
		let socks_proxy_addr = SocketAddr::V4(
			socks_proxy_addr
				.parse()
				.map_err(|e| Error::Config(format!("{:?}", e)))?,
		);
		info!(
			"Starting external Tor process to send at {:?}.",
			socks_proxy_addr
		);

		let mut hm_tor_bridge: HashMap<String, String> = HashMap::new();
		let bridge_config = config.bridge_config();
		if bridge_config.bridge_line.is_some() {
			let bridge_struct = TorBridge::try_from(bridge_config.clone())
				.map_err(|e| Error::Config(format!("{:?}", e)))?;
			hm_tor_bridge = bridge_struct
				.to_hashmap()
				.map_err(|e| Error::Config(format!("{:?}", e)))?;
		}

		let mut hm_tor_proxy: HashMap<String, String> = HashMap::new();
		let proxy_config = config.proxy_config();
		if proxy_config.transport.is_some() || proxy_config.allowed_port.is_some() {
			let proxy = TorProxy::try_from(proxy_config.clone())
				.map_err(|e| Error::Config(format!("{:?}", e)))?;
			hm_tor_proxy = proxy
				.to_hashmap()
				.map_err(|e| Error::Config(format!("{:?}", e)))?;
		}

		output_tor_sender_config(
			tor_dir.to_str().unwrap(),
			socks_proxy_addr.to_string().as_str(),
			hm_tor_bridge,
			hm_tor_proxy,
		)
		.map_err(|e| Error::Config(format!("{:?}", e)))?;
		// Start TOR process
		let mut path = tor_dir.to_path_buf();
		path.push("torrc");
		tor.torrc_path(path.to_str().unwrap())
			.working_dir(tor_dir.to_str().unwrap())
			.timeout(
				config
					.bootstrap_timeout()
					.unwrap_or(Duration::from_secs(BOOTSTRAP_TIMEOUT_SECS))
					.as_secs(),
			)
			.completion_percent(100)
			.launch()
			.map_err(|e| Error::Generic(format!("{:?}", e)))?;
		Ok(tor)
	}

	fn get_process(&mut self, pid: i32) -> Option<&Process> {
		self.sys.refresh_all();
		self.sys.process((pid as usize).into())
	}

	pub fn tor_cmd(&mut self, tor_cmd: &str) -> &mut Self {
		self.tor_cmd = tor_cmd.to_string();
		self
	}

	pub fn torrc_path(&mut self, torrc_path: &str) -> &mut Self {
		self.torrc_path = Some(torrc_path.to_string());
		self
	}

	pub fn arg(&mut self, arg: String) -> &mut Self {
		self.args.push(arg);
		self
	}

	pub fn args(&mut self, args: Vec<String>) -> &mut Self {
		for arg in args {
			self.arg(arg);
		}
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
		let mut tor = Command::new(&self.tor_cmd);

		if let Some(ref d) = self.working_dir {
			tor.current_dir(d);
			let pid_file_name = format!("{}{}pid", d, MAIN_SEPARATOR);
			// kill off PID if its already running
			if Path::new(&pid_file_name).exists() {
				let pid = fs::read_to_string(&pid_file_name).map_err(Error::IO)?;
				let pid = pid
					.parse::<i32>()
					.map_err(|err| Error::PID(format!("{:?}", err)))?;
				if let Some(p) = self.get_process(pid) {
					let _ = p.kill();
				}
			}
		}
		if let Some(ref torrc_path) = self.torrc_path {
			tor.args(vec!["-f", torrc_path]);
		}
		let mut tor_process = tor
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                let msg = format!("TOR executable (`{}`) not found. Please ensure TOR is installed and on the path: {:?}", TOR_EXE_NAME, err);
                Error::Process(msg)
            })?;

		if let Some(ref d) = self.working_dir {
			// Split out the process id, so if we don't exit cleanly
			// we can take it down on the next run
			let pid_file_name = format!("{}{}pid", d, MAIN_SEPARATOR);
			let mut file = File::create(pid_file_name).map_err(Error::IO)?;
			file.write_all(format!("{}", tor_process.id()).as_bytes())
				.map_err(Error::IO)?;
		}

		let stdout = BufReader::new(tor_process.stdout.take().unwrap());

		self.process = Some(tor_process);
		let completion_percent = self.completion_percent;

		let (stdout_tx, stdout_rx) = channel();
		let stdout_timeout_tx = stdout_tx.clone();

		let timer = timer::Timer::new();
		// Keep this below chrono's DateTime range used by timer.
		let timeout = chrono::Duration::seconds(cmp::min(self.timeout, u32::MAX as u64) as i64);
		let _guard = timer.schedule_with_delay(timeout, move || {
			stdout_timeout_tx.send(Err(Error::Timeout)).unwrap_or(());
		});
		let stdout_thread = thread::spawn(move || {
			stdout_tx
				.send(Self::parse_tor_stdout(stdout, completion_percent))
				.unwrap_or(());
		});
		match stdout_rx.recv().unwrap() {
			Ok(stdout) => {
				stdout_thread.join().unwrap();
				self.stdout = Some(stdout);
				Ok(self)
			}
			Err(err) => {
				self.kill().unwrap_or(());
				stdout_thread.join().unwrap();
				Err(err)
			}
		}
	}

	fn parse_tor_stdout(
		mut stdout: BufReader<ChildStdout>,
		completion_perc: u8,
	) -> Result<BufReader<ChildStdout>, Error> {
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
								break;
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
		Ok(stdout)
	}

	pub fn kill(&mut self) -> Result<(), Error> {
		if let Some(ref mut process) = self.process {
			Ok(process
				.kill()
				.map_err(|err| Error::Process(format!("{}", err)))?)
		} else {
			Err(Error::ProcessNotStarted)
		}
	}
}

impl Drop for TorProcess {
	// kill the child
	fn drop(&mut self) {
		debug!("Dropping TOR process");
		self.kill().unwrap_or(());
	}
}
