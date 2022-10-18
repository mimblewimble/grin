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

	assert_eq!(
		x,
		Capabilities::HEADER_HIST
			| Capabilities::TXHASHSET_HIST
			| Capabilities::PEER_LIST
			| Capabilities::TX_KERNEL_HASH
			| Capabilities::PIBD_HIST
			| Capabilities::PIBD_HIST_1
	);
}
