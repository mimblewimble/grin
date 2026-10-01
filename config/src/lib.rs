#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::legacy_numeric_constants)]
#![allow(clippy::module_inception)]
#![allow(clippy::manual_flatten)]
#![allow(clippy::non_canonical_partial_ord_impl)]
#![allow(clippy::manual_try_fold)]
#![allow(clippy::useless_conversion)]
#![allow(clippy::new_without_default)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::explicit_counter_loop)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::suspicious_open_options)]
#![allow(clippy::unnecessary_unwrap)]
#![allow(clippy::while_let_loop)]
#![allow(clippy::arc_with_non_send_sync)]
#![allow(clippy::redundant_pattern_matching)]
#![allow(clippy::single_range_in_vec_init)]
#![allow(clippy::vec_init_then_push)]
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

//! Crate wrapping up the Grin binary and configuration file

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

#[macro_use]
extern crate serde_derive;

use grin_core as core;
use grin_p2p as p2p;
use grin_servers as servers;
use grin_util as util;

mod comments;
pub mod config;
pub mod types;

pub use crate::config::{initial_chain_type, initial_setup_server, load_server_config};
pub use crate::types::{ConfigError, ConfigMembers, GlobalConfig};
