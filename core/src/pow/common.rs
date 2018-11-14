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

//! Common types and traits for cuckoo/cuckatoo family of solvers

use blake2::blake2b::blake2b;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use pow::error::{Error, ErrorKind};
use pow::num::{PrimInt, ToPrimitive};
use pow::siphash::siphash24;
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
pub struct Edge<T>
where
	T: EdgeType,
{
	pub u: T,
	pub v: T,
}

impl<T> fmt::Display for Edge<T>
where
	T: EdgeType,
{
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(
			f,
			"(u: {}, v: {})",
			self.u.to_u64().unwrap_or(0),
			self.v.to_u64().unwrap_or(0)
		)
	}
}

/// An element of an adjencency list
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Link<T>
where
	T: EdgeType,
{
	pub next: T,
	pub to: T,
}

impl<T> fmt::Display for Link<T>
where
	T: EdgeType,
{
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(
			f,
			"(next: {}, to: {})",
			self.next.to_u64().unwrap_or(0),
			self.to.to_u64().unwrap_or(0)
		)
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

/// Macros to clean up integer unwrapping
#[macro_export]
macro_rules! to_u64 {
	($n:expr) => {
		$n.to_u64().ok_or(ErrorKind::IntegerCast)?
	};
}

#[macro_export]
macro_rules! to_u32 {
	($n:expr) => {
		$n.to_u64().ok_or(ErrorKind::IntegerCast)? as u32
	};
}

#[macro_export]
macro_rules! to_usize {
	($n:expr) => {
		$n.to_u64().ok_or(ErrorKind::IntegerCast)? as usize
	};
}

#[macro_export]
macro_rules! to_edge {
	($n:expr) => {
		T::from($n).ok_or(ErrorKind::IntegerCast)?
	};
}

/// Utility struct to calculate commonly used Cuckoo parameters calculated
/// from header, nonce, edge_bits, etc.
pub struct CuckooParams<T>
where
	T: EdgeType,
{
	pub edge_bits: u8,
	pub proof_size: usize,
	pub num_edges: u64,
	pub siphash_keys: [u64; 4],
	pub edge_mask: T,
}

impl<T> CuckooParams<T>
where
	T: EdgeType,
{
	/// Instantiates new params and calculate edge mask, etc
	pub fn new(edge_bits: u8, proof_size: usize) -> Result<CuckooParams<T>, Error> {
		let num_edges = (1 as u64) << edge_bits;
		let edge_mask = to_edge!(num_edges - 1);
		Ok(CuckooParams {
			edge_bits,
			proof_size,
			num_edges,
			siphash_keys: [0; 4],
			edge_mask,
		})
	}

	/// Reset the main keys used for siphash from the header and nonce
	pub fn reset_header_nonce(&mut self, header: Vec<u8>, nonce: Option<u32>) -> Result<(), Error> {
		self.siphash_keys = set_header_nonce(&header, nonce)?;
		Ok(())
	}

	/// Return siphash masked for type
	pub fn sipnode(&self, edge: T, uorv: u64, shift: bool) -> Result<T, Error> {
		let hash_u64 = siphash24(
			&self.siphash_keys,
			2 * edge.to_u64().ok_or(ErrorKind::IntegerCast)? + uorv,
		);
		let mut masked = hash_u64 & self.edge_mask.to_u64().ok_or(ErrorKind::IntegerCast)?;
		if shift {
			masked <<= 1;
			masked |= uorv;
		}
		Ok(T::from(masked).ok_or(ErrorKind::IntegerCast)?)
	}
}
