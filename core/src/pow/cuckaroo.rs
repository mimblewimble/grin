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

//! Implementation of Cuckaroo Cycle, based on Cuckoo Cycle designed by
//! John Tromp. Ported to Rust from https://github.com/tromp/cuckoo.
//!
//! Cuckaroo is an ASIC-Resistant variation of Cuckoo (CuckARoo) that's
//! aimed at making the lean mining mode of Cuckoo extremely ineffective.
//! It is one of the 2 proof of works used in Grin (the other one being the
//! more ASIC friendly Cuckatoo).
//!
//! In Cuckaroo, edges are calculated by repeatedly hashing the seeds to
//! obtain blocks of values. Nodes are then extracted from those edges.

use pow::common::{CuckooParams, Edge, EdgeType};
use pow::error::{Error, ErrorKind};
use pow::{PoWContext, Proof};

use std::cmp;

/// Cuckatoo cycle context. Only includes the verifier for now.
pub struct CuckarooContext<T>
where
	T: EdgeType,
{
	params: CuckooParams<T>,
}

impl<T> PoWContext<T> for CuckarooContext<T>
where
	T: EdgeType,
{
	fn new(edge_bits: u8, proof_size: usize, max_sols: u32) -> Result<Box<Self>, Error> {
		let params = CuckooParams::new(edge_bits, proof_size)?;
		let num_edges = to_edge!(params.num_edges);
		Ok(Box::new(CuckarooContext { params }))
	}

	fn set_header_nonce(
		&mut self,
		header: Vec<u8>,
		nonce: Option<u32>,
		_solve: bool,
	) -> Result<(), Error> {
		self.params.reset_header_nonce(header, nonce)
	}

	fn find_cycles(&mut self) -> Result<Vec<Proof>, Error> {
		unimplemented!()
	}

	fn verify(&self, proof: &Proof) -> Result<(), Error> {
		unimplemented!()
	}
}
