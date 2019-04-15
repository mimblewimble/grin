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

//! Lightweight rebuildable view of the kernel MMR.
//! Used when receiving a "kernel data" file from a peer to
//! (re)build the kernel MMR locally.

use std::fs::File;
use std::io;
use std::io::{BufReader, Read};
use std::time::Duration;

use croaring::Bitmap;
use tempfile;
use tempfile::TempDir;

use crate::core::core::pmmr::{self, PMMR};
use crate::core::core::{BlockHeader, TxKernel, TxKernelEntry};
use crate::core::ser::{Readable, StreamingReader};
use crate::error::{Error, ErrorKind};
use crate::store::Batch;
use crate::txhashset::txhashset::{PMMRHandle, TxHashSet};
use grin_store::pmmr::PMMRBackend;

/// Rebuildable kernel view backed by a tempdir.
pub struct RebuildableKernelView<'a> {
	pmmr: PMMR<'a, TxKernel, PMMRBackend<TxKernel>>,
}

impl<'a> RebuildableKernelView<'a> {
	pub fn new(backend: &'a mut PMMRBackend<TxKernel>) -> RebuildableKernelView<'a> {
		RebuildableKernelView {
			pmmr: PMMR::at(backend, 0),
		}
	}

	pub fn truncate(&mut self) -> Result<(), Error> {
		debug!("Truncating temp kernel view.");
		self.pmmr
			.rewind(0, &Bitmap::create())
			.map_err(&ErrorKind::TxHashSetErr)?;
		Ok(())
	}

	pub fn rebuild(
		&mut self,
		data: &mut Read,
		txhashset: &TxHashSet,
		header: &BlockHeader,
	) -> Result<(), Error> {
		// Rebuild is all-or-nothing. Truncate everything before we begin.
		self.truncate()?;

		let mut stream = StreamingReader::new(data, Duration::from_secs(1));

		let mut current_pos = 0;
		let mut current_header = txhashset.get_header_by_height(0)?;

		loop {
			while current_pos < current_header.kernel_mmr_size {
				// Read and verify the next kernel from the stream of data.
				let kernel = TxKernelEntry::read(&mut stream)?;
				kernel.kernel.verify()?;

				// Apply it to the MMR and keep track of last_pos.
				let (_, last_pos) = self.apply_kernel(&kernel.kernel)?;
				current_pos = last_pos;
			}

			// Verify the kernel MMR root is correct for current header.
			let root = self.pmmr.root();
			if root != current_header.kernel_root {
				return Err(ErrorKind::InvalidTxHashSet(format!(
					"Kernel root at {} does not match",
					current_header.height
				))
				.into());
			}

			// Periodically sync the PMMR backend as we rebuild it.
			if current_header.height % 1000 == 0 {
				self.pmmr
					.sync()
					.map_err(|_| ErrorKind::TxHashSetErr("failed to sync pmmr".into()))?;
				debug!(
					"Rebuilt kernel MMR to height: {}, kernels: {} (MMR size: {}) ...",
					current_header.height,
					pmmr::n_leaves(self.pmmr.last_pos),
					self.pmmr.last_pos,
				);
			}

			// Done if we have reached the specified header.
			if current_header == *header {
				break;
			} else if current_header.height >= header.height {
				return Err(ErrorKind::InvalidTxHashSet(format!(
					"Header mismatch when rebuilding kernel MMR.",
				))
				.into());
			} else {
				current_header = txhashset.get_header_by_height(current_header.height + 1)?;
			}
		}

		// One final sync to ensure everything is saved (to the tempdir).
		self.pmmr
			.sync()
			.map_err(|_| ErrorKind::TxHashSetErr("failed to sync pmmr".into()))?;
		debug!(
			"Rebuilt kernel MMR to height: {}, kernels: {} (MMR size: {}) DONE",
			current_header.height,
			pmmr::n_leaves(self.pmmr.last_pos),
			self.pmmr.last_pos,
		);

		Ok(())
	}

	/// Push kernel onto MMR (hash, data and size files).
	/// Returns the pos of the element applies and "last_pos" including all new parents.
	pub fn apply_kernel(&mut self, kernel: &TxKernel) -> Result<(u64, u64), Error> {
		let pos = self.pmmr.push(kernel).map_err(&ErrorKind::TxHashSetErr)?;
		Ok(pos)
	}
}
