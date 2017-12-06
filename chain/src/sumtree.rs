
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
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use util::secp::pedersen::{RangeProof, Commitment};

use core::core::{Block, SumCommit, TxKernel};
use core::core::pmmr::{Backend, HashSum, NoSum, Summable, PMMR};
use core::core::hash::Hashed;
use grin_store;
use grin_store::sumtree::PMMRBackend;
use types::ChainStore;
use types::Error;
use util::LOGGER;

const SUMTREES_SUBDIR: &'static str = "sumtrees";
const UTXO_SUBDIR: &'static str = "utxo";
const RANGE_PROOF_SUBDIR: &'static str = "rangeproof";
const KERNEL_SUBDIR: &'static str = "kernel";

struct PMMRHandle<T>
where
	T: Summable + Clone,
{
	backend: PMMRBackend<T>,
	last_pos: u64,
}

impl<T> PMMRHandle<T>
where
	T: Summable + Clone,
{
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
/// kernels. Also handles the index of Commitments to positions in the
/// output and range proof sum trees.
///
/// Note that the index is never authoritative, only the trees are
/// guaranteed to indicate whether an output is spent or not. The index
/// may have commitments that have already been spent, even with
/// pruning enabled.
pub struct SumTrees {
	output_pmmr_h: PMMRHandle<SumCommit>,
	rproof_pmmr_h: PMMRHandle<NoSum<RangeProof>>,
	kernel_pmmr_h: PMMRHandle<NoSum<TxKernel>>,

	// chain store used as index of commitments to MMR positions
	commit_index: Arc<ChainStore>,
}

impl SumTrees {
	/// Open an existing or new set of backends for the SumTrees
	pub fn open(root_dir: String, commit_index: Arc<ChainStore>) -> Result<SumTrees, Error> {
		Ok(SumTrees {
			output_pmmr_h: PMMRHandle::new(root_dir.clone(), UTXO_SUBDIR)?,
			rproof_pmmr_h: PMMRHandle::new(root_dir.clone(), RANGE_PROOF_SUBDIR)?,
			kernel_pmmr_h: PMMRHandle::new(root_dir.clone(), KERNEL_SUBDIR)?,
			commit_index: commit_index,
		})
	}

	/// Whether a given commitment exists in the Output MMR and it's unspent
	pub fn is_unspent(&self, commit: &Commitment) -> Result<bool, Error> {
		let rpos = self.commit_index.get_output_pos(commit);
		match rpos {
			Ok(pos) => {
				// checking the position is within the MMR, the commit index could be
				// returning rewound data
				if pos > self.output_pmmr_h.last_pos {
					Ok(false)
				} else {
					Ok(self.output_pmmr_h.backend.get(pos).is_some())
				}
			}
			Err(grin_store::Error::NotFoundErr) => Ok(false),
			Err(e) => Err(Error::StoreErr(e, "sumtree unspent check".to_owned())),
		}
	}

	/// returns the last N nodes inserted into the tree (i.e. the 'bottom'
	/// nodes at level 0
	pub fn last_n_utxo(&mut self, distance: u64) -> Vec<HashSum<SumCommit>> {
		let output_pmmr = PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
		output_pmmr.get_last_n_insertions(distance)
	}

	/// as above, for range proofs
	pub fn last_n_rangeproof(&mut self, distance: u64) -> Vec<HashSum<NoSum<RangeProof>>> {
		let rproof_pmmr = PMMR::at(&mut self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos);
		rproof_pmmr.get_last_n_insertions(distance)
	}

	/// as above, for kernels
	pub fn last_n_kernel(&mut self, distance: u64) -> Vec<HashSum<NoSum<TxKernel>>> {
		let kernel_pmmr = PMMR::at(&mut self.kernel_pmmr_h.backend, self.kernel_pmmr_h.last_pos);
		kernel_pmmr.get_last_n_insertions(distance)
	}

	/// Get sum tree roots
	pub fn roots(
		&mut self,
	) -> (
		HashSum<SumCommit>,
		HashSum<NoSum<RangeProof>>,
		HashSum<NoSum<TxKernel>>,
	) {
		let output_pmmr = PMMR::at(&mut self.output_pmmr_h.backend, self.output_pmmr_h.last_pos);
		let rproof_pmmr = PMMR::at(&mut self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos);
		let kernel_pmmr = PMMR::at(&mut self.kernel_pmmr_h.backend, self.kernel_pmmr_h.last_pos);
		(output_pmmr.root(), rproof_pmmr.root(), kernel_pmmr.root())
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
where
	F: FnOnce(&mut Extension) -> Result<T, Error>,
{
	let sizes: (u64, u64, u64);
	let res: Result<T, Error>;
	let rollback: bool;
	{
		debug!(LOGGER, "Starting new sumtree extension.");
		let commit_index = trees.commit_index.clone();
		let mut extension = Extension::new(trees, commit_index);
		res = inner(&mut extension);
		rollback = extension.rollback;
		if res.is_ok() && !rollback {
			extension.save_pos_index()?;
		}
		sizes = extension.sizes();
	}
	match res {
		Err(e) => {
			debug!(LOGGER, "Error returned, discarding sumtree extension.");
			trees.output_pmmr_h.backend.discard();
			trees.rproof_pmmr_h.backend.discard();
			trees.kernel_pmmr_h.backend.discard();
			Err(e)
		}
		Ok(r) => {
			if rollback {
				debug!(LOGGER, "Rollbacking sumtree extension.");
				trees.output_pmmr_h.backend.discard();
				trees.rproof_pmmr_h.backend.discard();
				trees.kernel_pmmr_h.backend.discard();
			} else {
				debug!(LOGGER, "Committing sumtree extension.");
				trees.output_pmmr_h.backend.sync()?;
				trees.rproof_pmmr_h.backend.sync()?;
				trees.kernel_pmmr_h.backend.sync()?;
				trees.output_pmmr_h.last_pos = sizes.0;
				trees.rproof_pmmr_h.last_pos = sizes.1;
				trees.kernel_pmmr_h.last_pos = sizes.2;
			}

			debug!(LOGGER, "Sumtree extension done.");
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

	commit_index: Arc<ChainStore>,
	new_output_commits: HashMap<Commitment, u64>,
	new_kernel_excesses: HashMap<Commitment, u64>,
	rollback: bool,
}

impl<'a> Extension<'a> {
	// constructor
	fn new(trees: &'a mut SumTrees, commit_index: Arc<ChainStore>) -> Extension<'a> {
		Extension {
			output_pmmr: PMMR::at(
				&mut trees.output_pmmr_h.backend,
				trees.output_pmmr_h.last_pos,
			),
			rproof_pmmr: PMMR::at(
				&mut trees.rproof_pmmr_h.backend,
				trees.rproof_pmmr_h.last_pos,
			),
			kernel_pmmr: PMMR::at(
				&mut trees.kernel_pmmr_h.backend,
				trees.kernel_pmmr_h.last_pos,
			),
			commit_index: commit_index,
			new_output_commits: HashMap::new(),
			new_kernel_excesses: HashMap::new(),
			rollback: false,
		}
	}

	/// Apply a new set of blocks on top the existing sum trees. Blocks are
	/// applied in order of the provided Vec. If pruning is enabled, inputs also
	/// prune MMR data.
	pub fn apply_block(&mut self, b: &Block) -> Result<(), Error> {

		// doing inputs first guarantees an input can't spend an output in the
  // same block, enforcing block cut-through
		for input in &b.inputs {
			let pos_res = self.commit_index.get_output_pos(&input.commitment());
			if let Ok(pos) = pos_res {
				match self.output_pmmr.prune(pos, b.header.height as u32) {
					Ok(true) => {
						self.rproof_pmmr
							.prune(pos, b.header.height as u32)
							.map_err(|s| Error::SumTreeErr(s))?;
					}
					Ok(false) => return Err(Error::AlreadySpent),
					Err(s) => return Err(Error::SumTreeErr(s)),
				}
			} else {
				return Err(Error::SumTreeErr(
					format!("Missing index for {:?}", input.commitment()),
				));
			}
		}

		// checking any position after the MMR size is useless, catches rewind
		// edge cases
		let output_max_index = self.output_pmmr.unpruned_size();
		let kernel_max_index = self.kernel_pmmr.unpruned_size();

		for out in &b.outputs {
			let commit = out.commitment();
			if let Ok(pos) = self.commit_index.get_output_pos(&commit) {
				if pos <= output_max_index {
					// we need to check whether the commitment is in the current MMR view
					// as well as the index doesn't support rewind and is non-authoritative
					// (non-historical node will have a much smaller one)
					// note that this doesn't show the commitment *never* existed, just
					// that this is not an existing unspent commitment right now
					if let Some(c) = self.output_pmmr.get(pos) {
						let hashsum = HashSum::from_summable(
							pos, &SumCommit{commit}, Some(out.switch_commit_hash));
						// as we're processing a new fork, we may get a position on the old
						// fork that exists but matches a different node, filtering that
						// case out
						if c.hash == hashsum.hash {
							return Err(Error::DuplicateCommitment(out.commitment()));
						}
					}
				}
			}
			// push new outputs commitments in their MMR and save them in the index
			let pos = self.output_pmmr
				.push(
					SumCommit {
						commit: out.commitment(),
					},
					Some(out.switch_commit_hash()),
				)
				.map_err(&Error::SumTreeErr)?;

			self.new_output_commits.insert(out.commitment(), pos);

			// push range proofs in their MMR
			self.rproof_pmmr
				.push(NoSum(out.proof), None::<RangeProof>)
				.map_err(&Error::SumTreeErr)?;
		}

		for kernel in &b.kernels {
			if let Ok(pos) = self.commit_index.get_kernel_pos(&kernel.excess) {
				if pos <= kernel_max_index {
					// same as outputs
					if let Some(k) = self.kernel_pmmr.get(pos) {
						let hashsum = HashSum::from_summable(
							pos, &NoSum(kernel), None::<RangeProof>);
						if k.hash == hashsum.hash {
							return Err(Error::DuplicateKernel(kernel.excess.clone()));
						}
					}
				}
			}
			// push kernels in their MMR
			let pos = self.kernel_pmmr
				.push(NoSum(kernel.clone()), None::<RangeProof>)
				.map_err(&Error::SumTreeErr)?;
			self.new_kernel_excesses.insert(kernel.excess, pos);
		}
		Ok(())
	}

	fn save_pos_index(&self) -> Result<(), Error> {
		for (commit, pos) in &self.new_output_commits {
			self.commit_index.save_output_pos(commit, *pos)?;
		}
		for (excess, pos) in &self.new_kernel_excesses {
			self.commit_index.save_kernel_pos(excess, *pos)?;
		}
		Ok(())
	}

	/// Rewinds the MMRs to the provided position, given the last output and
	/// last kernel of the block we want to rewind to.
	pub fn rewind(&mut self, block: &Block) -> Result<(), Error> {
		debug!(
			LOGGER,
			"Rewind sumtrees to header {} at {}",
			block.header.hash(),
			block.header.height,
		);

		let out_pos_rew = match block.outputs.last() {
			Some(output) => self.commit_index.get_output_pos(&output.commitment())?,
			None => 0,
		};

		let kern_pos_rew = match block.kernels.last() {
			Some(kernel) => self.commit_index.get_kernel_pos(&kernel.excess)?,
			None => 0,
		};

		debug!(
			LOGGER,
			"Rewind sumtrees to output pos: {}, kernel pos: {}",
			out_pos_rew,
			kern_pos_rew,
		);

		let height = block.header.height;

		self.output_pmmr
			.rewind(out_pos_rew, height as u32)
			.map_err(&Error::SumTreeErr)?;
		self.rproof_pmmr
			.rewind(out_pos_rew, height as u32)
			.map_err(&Error::SumTreeErr)?;
		self.kernel_pmmr
			.rewind(kern_pos_rew, height as u32)
			.map_err(&Error::SumTreeErr)?;

		self.dump(true);
		Ok(())
	}

	/// Current root hashes and sums (if applicable) for the UTXO, range proof
	/// and kernel sum trees.
	pub fn roots(
		&self,
	) -> (
		HashSum<SumCommit>,
		HashSum<NoSum<RangeProof>>,
		HashSum<NoSum<TxKernel>>,
	) {
		(
			self.output_pmmr.root(),
			self.rproof_pmmr.root(),
			self.kernel_pmmr.root(),
		)
	}

	/// Force the rollback of this extension, no matter the result
	pub fn force_rollback(&mut self) {
		self.rollback = true;
	}

	/// Dumps the state of the 3 sum trees to stdout for debugging. Short
	/// version
	/// only prints the UTXO tree.
	pub fn dump(&self, short: bool) {
		debug!(LOGGER, "-- outputs --");
		self.output_pmmr.dump(short);
		if !short {
			debug!(LOGGER, "-- range proofs --");
			self.rproof_pmmr.dump(short);
			debug!(LOGGER, "-- kernels --");
			self.kernel_pmmr.dump(short);
		}
	}

	// Sizes of the sum trees, used by `extending` on rollback.
	fn sizes(&self) -> (u64, u64, u64) {
		(
			self.output_pmmr.unpruned_size(),
			self.rproof_pmmr.unpruned_size(),
			self.kernel_pmmr.unpruned_size(),
		)
	}
}
