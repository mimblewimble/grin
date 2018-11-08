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

//! Library module for the key holder functionalities provided by Grin.

extern crate blake2_rfc as blake2;
extern crate byteorder;
#[macro_use]
extern crate grin_util as util;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate digest;
extern crate hmac;
extern crate log;
extern crate pbkdf2;
extern crate ripemd160;
extern crate serde_json;
extern crate sha2;
extern crate uuid;
#[macro_use]
extern crate lazy_static;

mod base58;
pub mod extkey_bip32;
pub mod mnemonic;
mod types;

pub mod keychain;
pub use extkey_bip32::ChildNumber;
pub use keychain::ExtKeychain;
pub use types::{
	BlindSum, BlindingFactor, Error, ExtKeychainPath, Identifier, Keychain, IDENTIFIER_SIZE,
};
