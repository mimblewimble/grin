//! Implementation of the MimbleWimble paper.
//! https://download.wpsoftware.net/bitcoin/wizardry/mimblewimble.txt

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

extern crate byteorder;
extern crate crypto;
extern crate rand;
extern crate secp256k1zkp as secp;
extern crate time;
extern crate tiny_keccak;

#[macro_use]
pub mod macros;

pub mod core;
pub mod genesis;
pub mod pow;
pub mod ser;
// mod chain;
