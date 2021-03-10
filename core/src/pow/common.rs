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

//! Common types and traits for cuckoo family of solvers

use crate::pow::error::Error;
use crate::pow::siphash::siphash24;
use blake2::blake2b::blake2b;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num::{PrimInt, ToPrimitive};
use std::fmt;
use std::hash::Hash;
use std::io::Cursor;
use std::ops::{BitOrAssign, Mul};

/// Operations needed for edge type (going to be u32 or u64)
pub trait EdgeType: PrimInt + ToPrimitive + Mul + BitOrAssign + Hash {}
impl EdgeType for u32 {}
impl EdgeType for u64 {}

/// An edge in the Cuckoo graph, simply references two u64 nodes.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct Edge {
	pub u: u64,
	pub v: u64,
}

impl fmt::Display for Edge {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "(u: {}, v: {})", self.u, self.v)
	}
}

/// An element of an adjencency list
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Link {
	pub next: u64,
	pub to: u64,
}

impl fmt::Display for Link {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "(next: {}, to: {})", self.next, self.to)
	}
}

pub fn set_header_nonce(header: &[u8], nonce: Option<u32>) -> Result<[u64; 4], Error> {
	if let Some(n) = nonce {
		let len = header.len();
		let mut header = header.to_owned();
		header.truncate(len - 4); // drop last 4 bytes (u32) off the end
		header.write_u32::<LittleEndian>(n)?;
		create_siphash_keys(&header)
	} else {
		create_siphash_keys(&header)
	}
}

pub fn create_siphash_keys(header: &[u8]) -> Result<[u64; 4], Error> {
	let h = blake2b(32, &[], &header);
	let hb = h.as_bytes();
	let mut rdr = Cursor::new(hb);
	Ok([
		rdr.read_u64::<LittleEndian>()?,
		rdr.read_u64::<LittleEndian>()?,
		rdr.read_u64::<LittleEndian>()?,
		rdr.read_u64::<LittleEndian>()?,
	])
}

/// Utility struct to calculate commonly used Cuckoo parameters calculated
/// from header, nonce, edge_bits, etc.
pub struct CuckooParams {
	pub edge_bits: u8,
	pub proof_size: usize,
	pub num_edges: u64,
	pub siphash_keys: [u64; 4],
	pub edge_mask: u64,
	pub node_mask: u64,
}

impl CuckooParams {
	/// Instantiates new params and calculate edge mask, etc
	pub fn new(edge_bits: u8, node_bits: u8, proof_size: usize) -> Result<CuckooParams, Error> {
		let num_edges = 1u64 << edge_bits;
		let edge_mask = num_edges - 1;
		let num_nodes = 1u64 << node_bits;
		let node_mask = num_nodes - 1;
		Ok(CuckooParams {
			edge_bits,
			proof_size,
			num_edges,
			siphash_keys: [0; 4],
			edge_mask,
			node_mask,
		})
	}

	/// Reset the main keys used for siphash from the header and nonce
	pub fn reset_header_nonce(&mut self, header: Vec<u8>, nonce: Option<u32>) -> Result<(), Error> {
		self.siphash_keys = set_header_nonce(&header, nonce)?;
		Ok(())
	}

	/// Return siphash masked for type
	pub fn sipnode(&self, edge: u64, uorv: u64) -> Result<u64, Error> {
		let hash_u64 = siphash24(&self.siphash_keys, 2 * edge + uorv);
		let node = hash_u64 & self.node_mask;
		Ok(node)
	}
}
