
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
use std::fs::File;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use util::{static_secp_instance};
use util::secp::pedersen::{RangeProof, Commitment};

use core::consensus::reward;
use core::core::{Block, BlockHeader, Input, Output, OutputIdentifier, 
	OutputFeatures, OutputStoreable, TxKernel};
use core::core::pmmr::{self, PMMR};
use core::core::hash::{Hash, Hashed};
use core::ser::{self, Writeable};
use grin_store;
use grin_store::pmmr::PMMRBackend;
use grin_store::flatfile::FlatFileStore;
use types::ChainStore;
use types::Error;
use util::{LOGGER, zip};

const SUMTREES_SUBDIR: &'static str = "sumtrees";
const UTXO_SUBDIR: &'static str = "utxo";
const RANGE_PROOF_SUBDIR: &'static str = "rangeproof";
const KERNEL_SUBDIR: &'static str = "kernel";
const SUMTREES_ZIP: &'static str = "sumtrees_snapshot.zip";

const TEMP_RANGEPROOF_SIZE:usize = 5134 + 1;

struct PMMRHandle<T>
where
	T: Writeable,
{
	backend: PMMRBackend<T>,
	last_pos: u64,
}

impl<T> PMMRHandle<T>
where
	T: Writeable,
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
/// output and range proof pmmr trees.
///
/// Note that the index is never authoritative, only the trees are
/// guaranteed to indicate whether an output is spent or not. The index
/// may have commitments that have already been spent, even with
/// pruning enabled.

pub struct SumTrees {
	utxo_pmmr_h: PMMRHandle<OutputStoreable>,
	rproof_pmmr_h: PMMRHandle<RangeProof>,
	kernel_pmmr_h: PMMRHandle<TxKernel>,

	utxo_file: FlatFileStore<OutputStoreable>,
	rproof_file: FlatFileStore<RangeProof>,
	kernel_file: FlatFileStore<TxKernel>,

	// chain store used as index of commitments to MMR positions
	commit_index: Arc<ChainStore>,
}

impl SumTrees {
	/// Open an existing or new set of backends for the SumTrees
	pub fn open(root_dir: String, commit_index: Arc<ChainStore>) -> Result<SumTrees, Error> {

		let utxo_file_path: PathBuf = [&root_dir, SUMTREES_SUBDIR, UTXO_SUBDIR].iter().collect();
		fs::create_dir_all(utxo_file_path.clone())?;
		let utxo_file = FlatFileStore::new(utxo_file_path.to_str().unwrap().to_owned(), OutputStoreable::size())?;

		let rproof_file_path: PathBuf = [&root_dir, SUMTREES_SUBDIR, RANGE_PROOF_SUBDIR].iter().collect();
		fs::create_dir_all(rproof_file_path.clone())?;
		//Constant proof size + length just temporary for now
		let rproof_file = FlatFileStore::new(rproof_file_path.to_str().unwrap().to_owned(), TEMP_RANGEPROOF_SIZE)?;

		let kernel_file_path: PathBuf = [&root_dir, SUMTREES_SUBDIR, KERNEL_SUBDIR].iter().collect();
		fs::create_dir_all(kernel_file_path.clone())?;
		let kernel_file = FlatFileStore::new(kernel_file_path.to_str().unwrap().to_owned(), TxKernel::size())?;

		Ok(SumTrees {
			utxo_pmmr_h: PMMRHandle::new(root_dir.clone(), UTXO_SUBDIR)?,
			rproof_pmmr_h: PMMRHandle::new(root_dir.clone(), RANGE_PROOF_SUBDIR)?,
			kernel_pmmr_h: PMMRHandle::new(root_dir.clone(), KERNEL_SUBDIR)?,
			utxo_file: utxo_file,
			rproof_file: rproof_file,
			kernel_file: kernel_file,
			commit_index: commit_index,
		})
	}

	/// Check is an output is unspent.
	/// We look in the index to find the output MMR pos.
	/// Then we check the entry in the output MMR and confirm the hash matches.
	pub fn is_unspent(&mut self, output: &OutputIdentifier) -> Result<(), Error> {
		match self.commit_index.get_output_pos(&output.commit) {
			Ok(pos) => {
				let output_pmmr:PMMR<OutputStoreable, _> = PMMR::at(
					&mut self.utxo_pmmr_h.backend,
					self.utxo_pmmr_h.last_pos,
				);
				if let Some(hash) = output_pmmr.get(pos) {
					let output = read_element_at_pmmr_index(&self.utxo_file, pos).unwrap().to_output();
					if hash == output.hash() {
						Ok(())
					} else {
						Err(Error::SumTreeErr(format!("sumtree hash mismatch")))
					}
				} else {
					Err(Error::OutputNotFound)
				}
			}
			Err(grin_store::Error::NotFoundErr) => Err(Error::OutputNotFound),
			Err(e) => Err(Error::StoreErr(e, format!("sumtree unspent check"))),
		}
	}

	/// Check the output being spent by the input has sufficiently matured.
	/// This only applies for coinbase outputs being spent (1,000 blocks).
	/// Non-coinbase outputs will always pass this check.
	/// For a coinbase output we find the block by the block hash provided in the input
	/// and check coinbase maturty based on the height of this block.
	pub fn is_matured(
		&mut self,
		input: &Input,
		height: u64,
	) -> Result<(), Error> {
		// We should never be in a situation where we are checking maturity rules
		// if the output is already spent (this should have already been checked).
		let output = OutputIdentifier::from_input(&input);
		assert!(self.is_unspent(&output).is_ok());

		// At this point we can be sure the input is spending the output
		// it claims to be spending, and that it is coinbase or non-coinbase.
		// If we are spending a coinbase output then go find the block
		// and check the coinbase maturity rule is being met.
		if input.features.contains(OutputFeatures::COINBASE_OUTPUT) {
			let block_hash = &input.out_block
				.expect("input spending coinbase output must have a block hash");
			let block = self.commit_index.get_block(&block_hash)?;
			block.verify_coinbase_maturity(&input, height)
				.map_err(|_| Error::ImmatureCoinbase)?;
		}
		Ok(())
	}

	/// returns the last N nodes inserted into the tree (i.e. the 'bottom'
	/// nodes at level 0
	/// TODO: These need to return the actual data from the flat-files instead of hashes now
	pub fn last_n_utxo(&mut self, distance: u64) -> Vec<Hash> {
		let utxo_pmmr:PMMR<OutputStoreable, _> = PMMR::at(&mut self.utxo_pmmr_h.backend, self.utxo_pmmr_h.last_pos);
		utxo_pmmr.get_last_n_insertions(distance)
	}

	/// as above, for range proofs
	pub fn last_n_rangeproof(&mut self, distance: u64) -> Vec<Hash> {
		let rproof_pmmr:PMMR<RangeProof, _> = PMMR::at(&mut self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos);
		rproof_pmmr.get_last_n_insertions(distance)
	}

	/// as above, for kernels
	pub fn last_n_kernel(&mut self, distance: u64) -> Vec<Hash> {
		let kernel_pmmr:PMMR<TxKernel, _> = PMMR::at(&mut self.kernel_pmmr_h.backend, self.kernel_pmmr_h.last_pos);
		kernel_pmmr.get_last_n_insertions(distance)
	}

	/// Output and kernel MMR indexes at the end of the provided block
	pub fn indexes_at(&self, block: &Block) -> Result<(u64, u64), Error> {
		indexes_at(block, self.commit_index.deref())
	}


	/// Get sum tree roots
	/// TODO: Return data instead of hashes
	pub fn roots(
		&mut self,
	) -> (
		Hash,
		Hash,
		Hash,
	) {
		let output_pmmr:PMMR<OutputStoreable, _> = PMMR::at(&mut self.utxo_pmmr_h.backend, self.utxo_pmmr_h.last_pos);
		let rproof_pmmr:PMMR<RangeProof, _> = PMMR::at(&mut self.rproof_pmmr_h.backend, self.rproof_pmmr_h.last_pos);
		let kernel_pmmr:PMMR<TxKernel, _> = PMMR::at(&mut self.kernel_pmmr_h.backend, self.kernel_pmmr_h.last_pos);
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
		let commit_index = trees.commit_index.clone();

		debug!(LOGGER, "Starting new sumtree extension.");
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
			trees.utxo_pmmr_h.backend.discard();
			trees.rproof_pmmr_h.backend.discard();
			trees.kernel_pmmr_h.backend.discard();
			trees.utxo_file.discard();
			trees.rproof_file.discard();
			trees.kernel_file.discard();
			Err(e)
		}
		Ok(r) => {
			if rollback {
				debug!(LOGGER, "Rollbacking sumtree extension.");
				trees.utxo_pmmr_h.backend.discard();
				trees.rproof_pmmr_h.backend.discard();
				trees.kernel_pmmr_h.backend.discard();
				trees.utxo_file.discard();
				trees.rproof_file.discard();
				trees.kernel_file.discard();
			} else {
				debug!(LOGGER, "Committing sumtree extension.");
				trees.utxo_pmmr_h.backend.sync()?;
				trees.rproof_pmmr_h.backend.sync()?;
				trees.kernel_pmmr_h.backend.sync()?;
				trees.utxo_file.sync()?;
				trees.rproof_file.sync()?;
				trees.kernel_file.sync()?;
				trees.utxo_pmmr_h.last_pos = sizes.0;
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
	utxo_pmmr: PMMR<'a, Output, PMMRBackend<OutputStoreable>>,
	rproof_pmmr: PMMR<'a, RangeProof, PMMRBackend<RangeProof>>,
	kernel_pmmr: PMMR<'a, TxKernel, PMMRBackend<TxKernel>>,

	utxo_file: &'a mut FlatFileStore<OutputStoreable>,
	rproof_file: &'a mut FlatFileStore<RangeProof>,
	kernel_file: &'a mut FlatFileStore<TxKernel>,
	commit_index: Arc<ChainStore>,
	new_output_commits: HashMap<Commitment, u64>,
	new_kernel_excesses: HashMap<Commitment, u64>,
	rollback: bool,
}

impl<'a> Extension<'a> {
	// constructor
	fn new(
		trees: &'a mut SumTrees,
		commit_index: Arc<ChainStore>,
	) -> Extension<'a> {

		Extension {
			utxo_pmmr: PMMR::at(
				&mut trees.utxo_pmmr_h.backend,
				trees.utxo_pmmr_h.last_pos,
			),
			rproof_pmmr: PMMR::at(
				&mut trees.rproof_pmmr_h.backend,
				trees.rproof_pmmr_h.last_pos,
			),
			kernel_pmmr: PMMR::at(
				&mut trees.kernel_pmmr_h.backend,
				trees.kernel_pmmr_h.last_pos,
			),
			utxo_file: &mut trees.utxo_file,
			rproof_file: &mut trees.rproof_file,
			kernel_file: &mut trees.kernel_file,
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

		// first applying coinbase outputs. due to the construction of PMMRs the
		// last element, when its a leaf, can never be pruned as it has no parent
		// yet and it will be needed to calculate that hash. to work around this,
		// we insert coinbase outputs first to add at least one output of padding
		for out in &b.outputs {
			if out.features.contains(OutputFeatures::COINBASE_OUTPUT) {
				self.apply_output(out)?;
			}
		}

		// then doing inputs guarantees an input can't spend an output in the
		// same block, enforcing block cut-through
		for input in &b.inputs {
			self.apply_input(input, b.header.height)?;
		}

		// now all regular, non coinbase outputs
		for out in &b.outputs {
			if !out.features.contains(OutputFeatures::COINBASE_OUTPUT) {
				self.apply_output(out)?;
			}
		}

		// finally, applying all kernels
		for kernel in &b.kernels {
			self.apply_kernel(kernel)?;
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

	fn apply_input(&mut self, input: &Input, height: u64) -> Result<(), Error> {
		let commit = input.commitment();
		let pos_res = self.get_output_pos(&commit);
		if let Ok(pos) = pos_res {
			if let Some(hash) = self.utxo_pmmr.get(pos) {
				// check hash from pmmr matches hash from input (or corresponding output)
				// if not then the input is not being honest about
				// what it is attempting to spend...
				let output = read_element_at_pmmr_index(self.utxo_file, pos).unwrap().to_output();
				if hash != output.hash() {
					return Err(Error::SumTreeErr(format!("output pmmr hash mismatch")));
				}

				// At this point we can be sure the input is spending the output
				// it claims to be spending, and it is coinbase or non-coinbase.
				// If we are spending a coinbase output then go find the block
				// and check the coinbase maturity rule is being met.
				if input.features.contains(OutputFeatures::COINBASE_OUTPUT) {
					let block_hash = &input.out_block
						.expect("input spending coinbase output must have a block hash");
					let block = self.commit_index.get_block(&block_hash)?;
					block.verify_coinbase_maturity(&input, height)
						.map_err(|_| Error::ImmatureCoinbase)?;
				}
			}

			// Now prune the utxo_pmmr, rproof_pmmr and their storage.
			// Input is not valid if we cannot prune successfully (to spend an unspent output).
			// TODO: rm log, skip list for utxo and range proofs
			match self.utxo_pmmr.prune(pos, height as u32) {
				Ok(true) => {
					self.rproof_pmmr
						.prune(pos, height as u32)
						.map_err(|s| Error::SumTreeErr(s))?;
				}
				Ok(false) => return Err(Error::AlreadySpent(commit)),
				Err(s) => return Err(Error::SumTreeErr(s)),
			}
		} else {
			return Err(Error::AlreadySpent(commit));
		}
		Ok(())
	}

	fn apply_output(&mut self, out: &Output) -> Result<(), Error> {
		let commit = out.commitment();

		if let Ok(pos) = self.get_output_pos(&commit) {
			// we need to check whether the commitment is in the current MMR view
			// as well as the index doesn't support rewind and is non-authoritative
			// (non-historical node will have a much smaller one)
			// note that this doesn't show the commitment *never* existed, just
			// that this is not an existing unspent commitment right now
			if let Some(c) = self.utxo_pmmr.get(pos) {
				// processing a new fork so we may get a position on the old
				// fork that exists but matches a different node
				// filtering that case out
				if c == out.hash() {
					return Err(Error::DuplicateCommitment(commit));
				}
			}
		}
		// push new outputs in their MMR and save them in the index
		let pos = self.utxo_pmmr
			.push(out.clone())
			.map_err(&Error::SumTreeErr)?;
		self.new_output_commits.insert(out.commitment(), pos);
		let _ = store_element(self.utxo_file, OutputStoreable::from_output(out));

		// push range proofs in their MMR and file
		self.rproof_pmmr
			.push(out.proof)
			.map_err(&Error::SumTreeErr)?;
		let _ = store_element(self.rproof_file, out.proof);
		Ok(())
	}

	fn apply_kernel(&mut self, kernel: &TxKernel) -> Result<(), Error> {
		if let Ok(pos) = self.get_kernel_pos(&kernel.excess) {
			// same as outputs
			if let Some(k) = self.kernel_pmmr.get(pos) {
				if k == kernel.hash() {
					return Err(Error::DuplicateKernel(kernel.excess.clone()));
				}
			}
		}

		// push kernels in their MMR and file
		let pos = self.kernel_pmmr
			.push(kernel.clone())
			.map_err(&Error::SumTreeErr)?;
		self.new_kernel_excesses.insert(kernel.excess, pos);
		let _ = store_element(self.kernel_file, kernel.clone());

		Ok(())
	}

	/// Rewinds the MMRs to the provided block, using the last output and
	/// last kernel of the block we want to rewind to.
	pub fn rewind(&mut self, block: &Block) -> Result<(), Error> {
		debug!(
			LOGGER,
			"Rewind sumtrees to header {} at {}",
			block.header.hash(),
			block.header.height,
		);

		// rewind each MMR
		let (out_pos_rew, kern_pos_rew) = indexes_at(block, self.commit_index.deref())?;
		self.rewind_pos(block.header.height, out_pos_rew, kern_pos_rew)?;
		
		// rewind the file stores
		let _ = rewind_to_pmmr_index(self.utxo_file, out_pos_rew);
		let _ = rewind_to_pmmr_index(self.rproof_file, out_pos_rew);
		// rewind the kernels
		// the number of kernels is the number of leaves in the MMR, which is the
		// sum of the number of leaf nodes under each peak in the MMR
		//let kern_pos: u64 = pmmr::peaks(kern_pos_rew).iter().map(|n| (1 << n) as u64).sum();
		let _ = rewind_to_pmmr_index(self.kernel_file, kern_pos_rew);

		Ok(())
	}

	/// Rewinds the MMRs to the provided positions, given the output and
	/// kernel we want to rewind to.
	pub fn rewind_pos(&mut self, height: u64, out_pos_rew: u64, kern_pos_rew: u64) -> Result<(), Error> {
		debug!(
			LOGGER,
			"Rewind sumtrees to output pos: {}, kernel pos: {}",
			out_pos_rew,
			kern_pos_rew,
		);

		self.utxo_pmmr
			.rewind(out_pos_rew, height as u32)
			.map_err(&Error::SumTreeErr)?;
		self.rproof_pmmr
			.rewind(out_pos_rew, height as u32)
			.map_err(&Error::SumTreeErr)?;
		self.kernel_pmmr
			.rewind(kern_pos_rew, height as u32)
			.map_err(&Error::SumTreeErr)?;

		Ok(())
	}

	fn get_output_pos(&self, commit: &Commitment) -> Result<u64, grin_store::Error> {
		if let Some(pos) = self.new_output_commits.get(commit) {
			Ok(*pos)
		} else {
			self.commit_index.get_output_pos(commit)
		}
	}

	fn get_kernel_pos(&self, excess: &Commitment) -> Result<u64, grin_store::Error> {
		if let Some(pos) = self.new_kernel_excesses.get(excess) {
			Ok(*pos)
		} else {
			self.commit_index.get_kernel_pos(excess)
		}
	}


	/// Current root hashes and sums (if applicable) for the UTXO, range proof
	/// and kernel sum trees.
	pub fn roots(
		&self,
	) -> (
		Hash,
		Hash,
		Hash,
	) {
		(
			self.utxo_pmmr.root(),
			self.rproof_pmmr.root(),
			self.kernel_pmmr.root(),
		)
	}

	/// Validate the current sumtree state against a block header
	pub fn validate(&self, header: &BlockHeader) -> Result<(), Error> {
		// validate all hashes and sums within the trees
		if let Err(e) = self.utxo_pmmr.validate() {
			return Err(Error::InvalidSumtree(e));
		}
		if let Err(e) = self.rproof_pmmr.validate() {
			return Err(Error::InvalidSumtree(e));
		}
		if let Err(e) = self.kernel_pmmr.validate() {
			return Err(Error::InvalidSumtree(e));
		}

		// validate the tree roots against the block header
		let (utxo_root, rproof_root, kernel_root) = self.roots();
		if utxo_root != header.utxo_root || rproof_root != header.range_proof_root
			|| kernel_root != header.kernel_root
		{
			return Err(Error::InvalidRoot);
		}

		// the real magicking: the sum of all kernel excess should equal the sum
		// of all UTXO commitments, minus the total supply
		let (kernel_sum, fees) = self.sum_kernels()?;
		let utxo_sum = self.sum_utxos()?;
		{
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			let over_commit = secp.commit_value(header.height * reward(0) - fees / 2)?;
			let adjusted_sum_utxo = secp.commit_sum(vec![utxo_sum], vec![over_commit])?;

			if adjusted_sum_utxo != kernel_sum {
				return Err(Error::InvalidSumtree("Differing UTXO commitment and kernel excess sums.".to_owned()));
			}
		}

		Ok(())
	}

	/// Rebuild the index of MMR positions to the corresponding UTXO and kernel
	/// by iterating over the whole MMR data. This is a costly operation
	/// performed only when we receive a full new chain state.
	pub fn rebuild_index(&self) -> Result<(), Error> {
		for n in 1..self.utxo_pmmr.unpruned_size()+1 {
			// non-pruned leaves only
			if pmmr::bintree_postorder_height(n) == 0 {
				if let Some(_) = self.utxo_pmmr.get(n) {
				  let out = read_element_at_pmmr_index(self.utxo_file, n).unwrap().to_output();
					self.commit_index.save_output_pos(&out.commit, n)?;
				}
			}
		}
		Ok(())
	}

	/// Force the rollback of this extension, no matter the result
	pub fn force_rollback(&mut self) {
		self.rollback = true;
	}

	/// Dumps the state of the 3 sum trees to stdout for debugging. Short
	/// version only prints the UTXO tree.
	pub fn dump(&self, short: bool) {
		debug!(LOGGER, "-- outputs --");
		self.utxo_pmmr.dump(short);
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
			self.utxo_pmmr.unpruned_size(),
			self.rproof_pmmr.unpruned_size(),
			self.kernel_pmmr.unpruned_size(),
		)
	}

	/// Sums the excess of all our kernels, validating their signatures on the way
	fn sum_kernels(&self) -> Result<(Commitment, u64), Error> {
		// make sure we have the right count of kernels using the MMR, the storage
		// file may have a few more
		let mmr_sz = self.kernel_pmmr.unpruned_size();
		let count: u64 = pmmr::peaks(mmr_sz).iter().map(|n| {
			(1 << pmmr::bintree_postorder_height(*n)) as u64
		}).sum();

		let mut kernel_file = File::open(self.kernel_file.data_file_path())?;
		let first: TxKernel = ser::deserialize(&mut kernel_file)?;
		first.verify()?;
		let mut sum_kernel = first.excess;
		let mut fees = first.fee;

		let secp = static_secp_instance();
		let mut kern_count = 1;
		loop {
			match ser::deserialize::<TxKernel>(&mut kernel_file) {
				Ok(kernel) => {
					kernel.verify()?;
					let secp = secp.lock().unwrap();
					sum_kernel = secp.commit_sum(vec![sum_kernel, kernel.excess], vec![])?;
					fees += kernel.fee;
					kern_count += 1;
					if kern_count == count {
						break;
					}
				}
				Err(_) => break,
			}
		}
		debug!(LOGGER, "Validated and summed {} kernels", kern_count);
		Ok((sum_kernel, fees))
	}

	/// Sums all our UTXO commitments
	fn sum_utxos(&self) -> Result<Commitment, Error> {
		let mut sum_utxo = None;
		let mut utxo_count = 0;
		let secp = static_secp_instance();
		for n in 1..self.utxo_pmmr.unpruned_size()+1 {
			if pmmr::bintree_postorder_height(n) == 0 {
				if let Some(output) = read_element_at_pmmr_index(self.utxo_file, n) {
					if n == 1 {
						sum_utxo = Some(output.commit);
					} else {
						let secp = secp.lock().unwrap();
						sum_utxo = Some(secp.commit_sum(vec![sum_utxo.unwrap(), output.commit], vec![])?);
					}
					utxo_count += 1;
				}
			}
		}
		debug!(LOGGER, "Summed {} UTXOs", utxo_count);
		Ok(sum_utxo.unwrap())
	}
}

fn store_element<T>(file_store: &mut FlatFileStore<T>, data: T)
	-> Result<(), String>
where
	T: ser::Readable + ser::Writeable + Clone
{
	file_store.append(vec![data])
}

fn read_element_at_pmmr_index<T>(file_store: &FlatFileStore<T>, pos: u64) -> Option<T>
where
	T: ser::Readable + ser::Writeable + Clone
{
	let leaf_index = pmmr::leaf_index(pos);
	// flat files are zero indexed
	file_store.get(leaf_index - 1)
}

fn remove_element_at_pmmr_index<T>(file_store: &mut FlatFileStore<T>, pos: u64) 
	-> Result<(), String>
where
	T: ser::Readable + ser::Writeable + Clone
{
	let leaf_index = pmmr::leaf_index(pos);
	// flat files are zero indexed
	file_store.remove(vec![leaf_index - 1])
}

fn rewind_to_pmmr_index<T>(file_store: &mut FlatFileStore<T>, pos: u64) -> Result<(), String>
where
	T: ser::Readable + ser::Writeable + Clone
{
	let leaf_index = pmmr::leaf_index(pos);
	// flat files are zero indexed
	file_store.rewind(leaf_index - 1)
}

/// Output and kernel MMR indexes at the end of the provided block
fn indexes_at(block: &Block, commit_index: &ChainStore) -> Result<(u64, u64), Error> {
	let out_idx = match block.outputs.last() {
		Some(output) => commit_index.get_output_pos(&output.commitment())
			.map_err(|e| {
				Error::StoreErr(e, format!("missing output pos for known block"))
			})?,
		None => 0,
	};

	let kern_idx = match block.kernels.last() {
		Some(kernel) => commit_index.get_kernel_pos(&kernel.excess)
			.map_err(|e| {
				Error::StoreErr(e, format!("missing kernel pos for known block"))
			})?,
		None => 0,
	};
	Ok((out_idx, kern_idx))
}

/// Packages the sumtree data files into a zip and returns a Read to the
/// resulting file
pub fn zip_read(root_dir: String) -> Result<File, Error> {
	let sumtrees_path = Path::new(&root_dir).join(SUMTREES_SUBDIR);
	let zip_path = Path::new(&root_dir).join(SUMTREES_ZIP);

	// create the zip archive
	{
		zip::compress(&sumtrees_path, &File::create(zip_path.clone())?)
			.map_err(|ze| Error::Other(ze.to_string()))?;
	}

	// open it again to read it back
	let zip_file = File::open(zip_path)?;
	Ok(zip_file)
}

/// Extract the sumtree data from a zip file and writes the content into the
/// sumtree storage dir
pub fn zip_write(root_dir: String, sumtree_data: File) -> Result<(), Error> {
	let sumtrees_path = Path::new(&root_dir).join(SUMTREES_SUBDIR);

	fs::create_dir_all(sumtrees_path.clone())?;
	zip::decompress(sumtree_data, &sumtrees_path)
			.map_err(|ze| Error::Other(ze.to_string()))
}
