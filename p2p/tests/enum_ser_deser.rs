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

extern crate grin_p2p as p2p;

extern crate enum_primitive;
extern crate num;

use num::FromPrimitive;

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
