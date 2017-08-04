// Copyright 2017 The Grin Developers
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

//! Values that should be shared across all modules, without necessarily
//! having to pass them all over the place, but aren't consensus values.
//! should be used sparingly.

/// An enum collecting sets of parameters used throughout the
/// code wherever mining is needed. This should allow for
/// different sets of parameters for different purposes,
/// e.g. CI, User testing, production values

use std::sync::{RwLock};

/// Define these here, as they should be developer-set, not really tweakable
/// by users

pub const AUTOMATED_TESTING_SIZESHIFT:u8 = 10;

pub const AUTOMATED_TESTING_PROOF_SIZE:usize = 4;

pub const USER_TESTING_SIZESHIFT:u8 = 16;

pub const USER_TESTING_PROOF_SIZE:usize = 42;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MiningParameterMode {
	/// For CI testing
	AutomatedTesting,

	/// For User testing
	UserTesting,

	/// For production, use the values in consensus.rs
	Production,
}

lazy_static!{
    pub static ref MINING_PARAMETER_MODE: RwLock<MiningParameterMode> = RwLock::new(MiningParameterMode::Production);
}

