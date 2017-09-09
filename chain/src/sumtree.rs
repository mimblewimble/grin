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

//! Utility structs to handle the 3 sumtrees (utxo, range proof, kernel) more
//! conveniently and transactionally.

use std::fs;
use std::path::Path;

use secp;
use secp::pedersen::RangeProof;

use core::core::{Block, TxKernel, SumCommit};
use core::core::pmmr::{Summable, NoSum, PMMR, HashSum};
use grin_store::sumtree::PMMRBackend;
use types::Error;

const SUMTREES_SUBDIR: &'static str = "sumtrees";
const UTXO_SUBDIR: &'static str = "utxo";
const RANGE_PROOF_SUBDIR: &'static str = "rangeproof";
const KERNEL_SUBDIR: &'static str = "kernel";

struct PMMRHandle<T> where T: Summable + Clone {
	backend: PMMRBackend<T>,
	last_pos: u64,
}

impl<T> PMMRHandle<T> where T: Summable + Clone {
	fn new(root_dir: String, file_name: &str) -> Result<PMMRHandle<T>, Error> {
		let path = Path::new(&root_dir).join(SUMTREES_SUBDIR).join(file_name);
		fs::create_dir_all(path.clone())?;
		let be = PMMRBackend::new(path.to_str().unwrap().to_string())?;
		let sz = be.unpruned_size()?;
		Ok(PMMRHandle {
			backend: be,
			last_pos: sz,
		})
	}
}

/// An easy to manipulate structure holding the 3 sum trees necessary to
/// validate blocks and capturing the UTXO set, the range proofs and the
/// kernels.
pub struct SumTrees {
	output_pmmr_h: PMMRHandle<SumCommit>,
	rproof_pmmr_h: PMMRHandle<NoSum<RangeProof>>,
	kernel_pmmr_h: PMMRHandle<NoSum<TxKernel>>,
}

impl SumTrees {
	/// Open an existing or new set of backends for the SumTrees
	pub fn open(root_dir: String) -> Result<SumTrees, Error> {
		Ok(SumTrees {
			output_pmmr_h: PMMRHandle::new(root_dir.clone(), UTXO_SUBDIR)?,
			rproof_pmmr_h: PMMRHandle::new(root_dir.clone(), RANGE_PROOF_SUBDIR)?,
			kernel_pmmr_h: PMMRHandle::new(root_dir.clone(), KERNEL_SUBDIR)?,
		})
	}
}

/// Starts a new unit of work to extend the chain with additional blocks,
/// accepting a closure that will work within that unit of work. The closure
/// has access to an Extension object that allows the addition of blocks to
/// the sumtrees and the checking of the current tree roots.
///
/// If the closure returns an error, modifications are canceled and the unit
/// of work is abandoned. Otherwise, the unit of work is permanently applied.
pub fn extending<'a, F, T>(trees: &'a mut SumTrees, inner: F) -> Result<T, Error>
	where F: FnOnce(&mut Extension) -> Result<T, Error> {
	
	let sizes: (u64, u64, u64);
	let res: Result<T, Error>;
	{
		let mut extension = Extension::new(trees);
		res = inner(&mut extension);
		sizes = extension.sizes();
	}
	match res {
		Err(e) => {
			trees.output_pmmr_h.backend.discard();
			trees.rproof_pmmr_h.backend.discard();
			trees.kernel_pmmr_h.backend.discard();
			Err(e)
		}
		Ok(r) => {
			trees.output_pmmr_h.backend.sync()?;
			trees.rproof_pmmr_h.backend.sync()?;
			trees.kernel_pmmr_h.backend.sync()?;
			trees.output_pmmr_h.last_pos = sizes.0;
			trees.rproof_pmmr_h.last_pos = sizes.1;
			trees.kernel_pmmr_h.last_pos = sizes.2;

			Ok(r)
		}
	}
}

/// Allows the application of new blocks on top of the sum trees in a
/// reversible manner within a unit of work provided by the `extending`
/// function.
pub struct Extension<'a> {
	output_pmmr: PMMR<'a, SumCommit, PMMRBackend<SumCommit>>,
	rproof_pmmr: PMMR<'a, NoSum<RangeProof>, PMMRBackend<NoSum<RangeProof>>>,
	kernel_pmmr: PMMR<'a, NoSum<TxKernel>, PMMRBackend<NoSum<TxKernel>>>,
}

impl<'a> Extension<'a> {

	// constructor
	fn new(trees: &'a mut SumTrees) -> Extension<'a> {
		Extension {
			output_pmmr: PMMR::at(&mut trees.output_pmmr_h.backend, trees.output_pmmr_h.last_pos),
			rproof_pmmr: PMMR::at(&mut trees.rproof_pmmr_h.backend, trees.rproof_pmmr_h.last_pos),
			kernel_pmmr: PMMR::at(&mut trees.kernel_pmmr_h.backend, trees.kernel_pmmr_h.last_pos),
		}
	}

	/// Apply a new set of blocks on top the existing sum trees. Blocks are
	/// applied in order of the provided Vec.
	pub fn apply_blocks(&mut self, blocks: Vec<&Block>) -> Result<(), Error> {
		let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
		for b in blocks {
			for out in &b.outputs {
				self.output_pmmr.push(SumCommit {
					commit: out.commitment(),
					secp: secp.clone(),
				}).map_err(&Error::SumTreeErr)?;
				self.rproof_pmmr.push(NoSum(out.proof)).map_err(&Error::SumTreeErr)?;
			}
			for kernel in &b.kernels {
				self.kernel_pmmr.push(NoSum(kernel.clone())).map_err(&Error::SumTreeErr)?;
			}
		}
		Ok(())
	}

	/// Current root hashes and sums (if applicable) for the UTXO, range proof
	/// and kernel sum trees.
	pub fn roots(&self) -> (HashSum<SumCommit>, HashSum<NoSum<RangeProof>>, HashSum<NoSum<TxKernel>>) {
		(self.output_pmmr.root(), self.rproof_pmmr.root(), self.kernel_pmmr.root())
	}

	// Sizes of the sum trees, used by `extending` on rollback.
	fn sizes(&self) -> (u64, u64, u64) {
		(self.output_pmmr.unpruned_size(), self.rproof_pmmr.unpruned_size(), self.kernel_pmmr.unpruned_size())
	}
}
