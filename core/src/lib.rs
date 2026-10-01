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

//! Implementation of the Mimblewimble paper.
//! https://download.wpsoftware.net/bitcoin/wizardry/mimblewimble.txt

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

#[macro_use]
extern crate enum_primitive;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate log;
#[macro_use]
pub mod macros;

pub mod consensus;
pub mod core;
pub mod genesis;
pub mod global;
pub mod libtx;
pub mod pow;
pub mod ser;
