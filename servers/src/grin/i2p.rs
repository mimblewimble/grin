// Copyright 2018 The Grin Developers
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

///! Handles initialization and configuration of the i2p daemon. This includes
///! the generation and management of i2p keys as well naming services.
use std::fs;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;

use i2p::net::{I2pAddr, I2pSocketAddr};
use i2p::sam::SamConnection;

use crate::common::types::ServerConfig;
use crate::p2p;

const I2P_DIR: &'static str = "i2p";
const PRIVKEY_FILE: &'static str = "priv";
const PUBKEY_FILE: &'static str = "pub";

/// Initializes I2P environment to be usable subsequently by the server, if
/// configured to. Will fail loudly if anything isn't where it's expected to
/// be.
pub fn init(config: &mut ServerConfig) -> Option<(I2pSocketAddr, Session)> {
	match &config.p2p_config.i2p_mode {
		p2p::I2pMode::Disabled => None,
		p2p::I2pMode::Enabled {
			autostart,
			exclusive: _,
			ref addr,
		} => {
			// Slight override of capabilities if i2p is enabled
			config.p2p_config.capabilities |= p2p::Capabilities::I2P_SUPPORTED;
			if *autostart {
				start_i2pd()
			}
			let (addr, privkey) = load_keys(addr, config);
			let session = Session::from_destination(addr, privkey);
			Some((addr, session))
		}
	}
}

fn start_i2pd() {
	unimplemented!()
}

fn load_keys(i2p_socket: &str, config: &ServerConfig) -> (I2pSocketAddr, String) {
	// TODO give I2P its own directory config?
	let mut i2p_root = PathBuf::from(&config.db_root);
	i2p_root.pop();
	i2p_root.push(I2P_DIR);
	let mut i2p_privkey = i2p_root.clone();
	i2p_privkey.push(PRIVKEY_FILE);
	let mut i2p_pubkey = i2p_root.clone();
	i2p_pubkey.push(PUBKEY_FILE);

	let (pubkey, privkey) = if i2p_pubkey.as_path().exists() && i2p_privkey.as_path().exists() {
		let privk = fs::read_to_string(i2p_privkey).expect("could not read i2p private key");
		let pubk = fs::read_to_string(i2p_pubkey).expect("could not read i2p public key");
		(pubk, privk)
	} else {
		let mut sam_conn = SamConnection::connect(i2p_socket).expect("Couldn't reach i2p daemon");
		let (pubk, privk) = sam_conn
			.generate_destination()
			.expect("Error generating i2p keys");
		fs::write(i2p_pubkey, &pubk).expect("Write error on i2p public key");
		fs::write(i2p_privkey, &privk).expect("Write error on i2p private key");
		(pubk, privk)
	};
	let i2p_sockaddr = I2pSocketAddr::new(
		I2pAddr::from_b64(&pubkey).unwrap(),
		i2p_socket.parse::<SocketAddr>().unwrap().port(),
	);
	(i2p_sockaddr, privkey)
}
