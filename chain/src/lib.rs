//! The block chain itself, validates and accepts new blocks, handles reorgs.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

#[macro_use]
extern crate bitflags;
extern crate byteorder;

#[macro_use(try_m)]
extern crate grin_core as core;
extern crate grin_store;
extern crate secp256k1zkp as secp;

pub mod pipe;
pub mod store;
pub mod types;

// Re-export the base interface

pub use types::ChainStore;
pub use pipe::Options;
pub use pipe::process_block;
