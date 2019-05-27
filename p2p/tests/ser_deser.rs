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

use grin_p2p as p2p;

use i2p::net::{I2pAddr, I2pSocketAddr};
use num::FromPrimitive;
use toml;

// Test that Healthy == 0.
#[test]
fn test_store_state_enum() {
	assert_eq!(p2p::State::from_i32(0), Some(p2p::State::Healthy));
}

#[test]
fn test_direction_enum() {
	assert_eq!(p2p::Direction::from_i32(0), Some(p2p::Direction::Inbound));
}

#[test]
fn test_reason_for_ban_enum() {
	assert_eq!(
		p2p::types::ReasonForBan::from_i32(0),
		Some(p2p::types::ReasonForBan::None)
	);
}

#[test]
fn test_type_enum() {
	assert_eq!(p2p::msg::Type::from_i32(0), Some(p2p::msg::Type::Error));
}

#[test]
fn test_peer_addr_enum() {
	let i2p_peer = p2p::PeerAddr::I2p(I2pSocketAddr::new(I2pAddr::new("some.i2p"), 9090));
	let b32_peer = p2p::PeerAddr::I2p(I2pSocketAddr::new(
		I2pAddr::new("7ygcsgwmstx4eil66vlaltb3usznxkkrd4dv425jf3qqrrflru3a.b32.i2p"),
		9090,
	));
	let sock_peer = p2p::PeerAddr::Socket("127.0.0.1:9090".to_string().parse().unwrap());

	let de_i2p_peer: p2p::PeerAddr =
		toml::from_str(toml::to_string(&i2p_peer.clone()).unwrap().as_str()).unwrap();
	let de_b32_peer: p2p::PeerAddr =
		toml::from_str(toml::to_string(&b32_peer.clone()).unwrap().as_str()).unwrap();
	let de_sock_peer: p2p::PeerAddr =
		toml::from_str(toml::to_string(&sock_peer.clone()).unwrap().as_str()).unwrap();

	assert_eq!(de_i2p_peer, i2p_peer);
	assert_eq!(de_b32_peer, b32_peer);
	assert_eq!(de_sock_peer, sock_peer);
}

#[test]
fn test_i2p_mode_enum() {
	let i2p_disabled = p2p::I2pMode::Disabled;
	let i2p_enabled = p2p::I2pMode::Enabled {
		autostart: false,
		exclusive: true,
		addr: "127.0.0.1:7656".to_string(),
	};

	let de_i2p_di: p2p::I2pMode =
		toml::from_str(toml::to_string(&i2p_disabled.clone()).unwrap().as_str()).unwrap();
	let de_i2p_en: p2p::I2pMode =
		toml::from_str(toml::to_string(&i2p_enabled.clone()).unwrap().as_str()).unwrap();

	assert_eq!(de_i2p_di, i2p_disabled);
	assert_eq!(de_i2p_en, i2p_enabled);
}

#[test]
fn test_capabilities() {
	assert_eq!(
		p2p::types::Capabilities::from_bits_truncate(0b00000000 as u32),
		p2p::types::Capabilities::UNKNOWN
	);
	assert_eq!(
		p2p::types::Capabilities::from_bits_truncate(0b10000000 as u32),
		p2p::types::Capabilities::UNKNOWN
	);

	assert_eq!(
		p2p::types::Capabilities::from_bits_truncate(0b11111 as u32),
		p2p::types::Capabilities::FULL_NODE
	);
	assert_eq!(
		p2p::types::Capabilities::from_bits_truncate(0b00011111 as u32),
		p2p::types::Capabilities::FULL_NODE
	);
	assert_eq!(
		p2p::types::Capabilities::from_bits_truncate(0b11111111 as u32),
		p2p::types::Capabilities::FULL_NODE
	);
	assert_eq!(
		p2p::types::Capabilities::from_bits_truncate(0b00111111 as u32),
		p2p::types::Capabilities::FULL_NODE
	);

	assert!(
		p2p::types::Capabilities::from_bits_truncate(0b00111111 as u32)
			.contains(p2p::types::Capabilities::FULL_NODE)
	);

	assert!(
		p2p::types::Capabilities::from_bits_truncate(0b00101111 as u32)
			.contains(p2p::types::Capabilities::TX_KERNEL_HASH)
	);
}
