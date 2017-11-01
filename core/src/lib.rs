// Copyright 2016 The Grin Developers
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

//! Implementation of the MimbleWimble paper.
//! https://download.wpsoftware.net/bitcoin/wizardry/mimblewimble.txt

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

#[macro_use]
extern crate bitflags;
extern crate blake2_rfc as blake2;
extern crate byteorder;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
#[macro_use]
extern crate lazy_static;
extern crate num_bigint as bigint;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate slog;
extern crate time;

#[macro_use]
pub mod macros;

pub mod consensus;
pub mod core;
pub mod genesis;
pub mod ser;
pub mod global;
