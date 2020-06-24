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

//! Lean miner for Cuckatoo Cycle

use croaring::Bitmap;

use crate::pow::common::CuckooParams;
use crate::pow::cuckatoo::CuckatooContext;
use crate::pow::error::Error;
use crate::pow::Proof;

/// Lean miner implementation aiming to be as short and simple as possible.
/// As a consequence, it's a little less than 10 times slower than John
/// Tromp's implementation, as it's not optimized for performance and reuses
/// croaring which is likely sub-optimal for this task.
pub struct Lean {
	params: CuckooParams,
	edges: Bitmap,
}

impl Lean {
	/// Instantiates a new lean miner based on some Cuckatoo parameters
	pub fn new(edge_bits: u8) -> Lean {
		// note that proof size doesn't matter to a lean miner
		let params = CuckooParams::new(edge_bits, edge_bits, 42).unwrap();

		// edge bitmap, before trimming all of them are on
		let mut edges = Bitmap::create_with_capacity(params.num_edges as u32);
		edges.flip_inplace(0..params.num_edges);

		Lean { params, edges }
	}

	/// Sets the header and nonce to seed the graph
	pub fn set_header_nonce(&mut self, header: Vec<u8>, nonce: u32) {
		self.params.reset_header_nonce(header, Some(nonce)).unwrap();
	}

	/// Trim edges in the Cuckatoo graph. This applies multiple trimming rounds
	/// and works well for Cuckatoo size above 18.
	pub fn trim(&mut self) {
		// trimming successively
		while self.edges.cardinality() > (7 * (self.params.num_edges >> 8) / 8) as u64 {
			self.count_and_kill();
		}
	}

	/// Finds the Cuckatoo Cycles on the remaining edges. Delegates the finding
	/// to a context, passing the trimmed edges iterator.
	pub fn find_cycles(&self, mut ctx: CuckatooContext) -> Result<Vec<Proof>, Error> {
		ctx.find_cycles_iter(self.edges.iter().map(|e| e as u64))
	}

	fn count_and_kill(&mut self) {
		// on each side u or v of the bipartite graph
		for uorv in 0..2 {
			let mut nodes = Bitmap::create();
			// increment count for each node
			for e in self.edges.iter() {
				let node = self.params.sipnode(e.into(), uorv).unwrap();
				nodes.add(node as u32);
			}

			// then kill edges with lone nodes (no neighbour at ^1)
			let mut to_kill = Bitmap::create();
			for e in self.edges.iter() {
				let node = self.params.sipnode(e.into(), uorv).unwrap();
				if !nodes.contains((node ^ 1) as u32) {
					to_kill.add(e);
				}
			}
			self.edges.andnot_inplace(&to_kill);
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use crate::global;
	use crate::pow::types::PoWContext;

	#[test]
	fn lean_miner() {
		global::set_local_chain_type(global::ChainTypes::Mainnet);
		let nonce = 15465723;
		let header = [0u8; 84].to_vec(); // with nonce
		let edge_bits = 19;

		let mut lean = Lean::new(edge_bits);
		lean.set_header_nonce(header.clone(), nonce);
		lean.trim();

		let mut ctx_u32 = CuckatooContext::new_impl(edge_bits, 42, 10).unwrap();
		ctx_u32.set_header_nonce(header, Some(nonce), true).unwrap();
		lean.find_cycles(ctx_u32).unwrap();
	}
}
