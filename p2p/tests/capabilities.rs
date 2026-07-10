// Copyright 2021 The Grin Developers
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

use grin_p2p::Capabilities;

// We use `contains()` to filter capabilities bits.
#[test]
fn capabilities_contains() {
	let x = Capabilities::HEADER_HIST;

	// capabilities contain themselves
	assert!(x.contains(Capabilities::HEADER_HIST));

	// UNKNOWN can be used to filter for any capabilities
	assert!(x.contains(Capabilities::UNKNOWN));

	// capabilities do not contain other disjoint capabilities
	assert_eq!(false, x.contains(Capabilities::PEER_LIST));
}

#[test]
fn default_capabilities() {
	let x = Capabilities::default();

	// Check that default capabilities is covered by UNKNOWN.
	assert!(x.contains(Capabilities::UNKNOWN));

	// Check that all the expected capabilities are included in default capabilities.
	assert!(x.contains(Capabilities::HEADER_HIST));
	assert!(x.contains(Capabilities::TXHASHSET_HIST));
	assert!(x.contains(Capabilities::PEER_LIST));
	assert!(x.contains(Capabilities::TX_KERNEL_HASH));
	assert!(x.contains(Capabilities::PIBD_HIST));
	assert!(x.contains(Capabilities::PIBD_HIST_1));
	assert!(x.contains(Capabilities::PIHD_HIST));

	assert_eq!(
		x,
		Capabilities::HEADER_HIST
			| Capabilities::TXHASHSET_HIST
			| Capabilities::PEER_LIST
			| Capabilities::TX_KERNEL_HASH
			| Capabilities::PIBD_HIST
			| Capabilities::PIBD_HIST_1
			| Capabilities::PIHD_HIST
	);

	// TLS is opt-in and not part of default capabilities.
	assert_eq!(false, x.contains(Capabilities::TLS));
}

#[test]
fn tls_capability_bit() {
	let tls = Capabilities::TLS;
	assert!(tls.contains(Capabilities::TLS));
	assert!(tls.contains(Capabilities::UNKNOWN));
	assert_eq!(false, tls.contains(Capabilities::PEER_LIST));

	let combined = Capabilities::PEER_LIST | Capabilities::TLS;
	assert!(combined.contains(Capabilities::PEER_LIST));
	assert!(combined.contains(Capabilities::TLS));
	assert!(combined.contains(Capabilities::PEER_LIST | Capabilities::TLS));
	assert_eq!(false, Capabilities::PEER_LIST.contains(combined));
}

#[test]
fn tls_peer_selection_helpers() {
	use grin_p2p::P2PConfig;

	let mut cfg = P2PConfig::default();
	assert_eq!(false, cfg.tls_peers_required());
	assert_eq!(
		cfg.peer_list_request_capabilities(),
		Capabilities::PEER_LIST
	);
	assert!(cfg.accepts_outbound_peer_capabilities(Capabilities::UNKNOWN));
	assert!(cfg.accepts_outbound_peer_capabilities(Capabilities::PEER_LIST));

	// tls_required alone does nothing without tls_enabled.
	cfg.tls_required = true;
	assert_eq!(false, cfg.tls_peers_required());

	cfg.tls_enabled = true;
	assert!(cfg.tls_peers_required());
	assert_eq!(
		cfg.peer_list_request_capabilities(),
		Capabilities::PEER_LIST | Capabilities::TLS
	);
	assert_eq!(
		false,
		cfg.accepts_outbound_peer_capabilities(Capabilities::UNKNOWN)
	);
	assert_eq!(
		false,
		cfg.accepts_outbound_peer_capabilities(Capabilities::PEER_LIST)
	);
	assert!(cfg.accepts_outbound_peer_capabilities(Capabilities::TLS));
	assert!(cfg.accepts_outbound_peer_capabilities(
		Capabilities::PEER_LIST | Capabilities::TLS
	));
}
