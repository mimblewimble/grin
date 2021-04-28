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

//! Lightweight readonly view into kernel MMR for convenience.

use crate::core::core::pmmr::{ReadablePMMR, ReadonlyPMMR, RewindablePMMR};
use crate::core::core::{BlockHeader, TxKernel};
use crate::error::Error;
use grin_store::pmmr::PMMRBackend;

/// Rewindable (but readonly) view of the kernel set (based on kernel MMR).
pub struct RewindableKernelView<'a> {
	pmmr: RewindablePMMR<'a, TxKernel, PMMRBackend<TxKernel>>,
	header: BlockHeader,
}

impl<'a> RewindableKernelView<'a> {
	/// Build a new readonly kernel view.
	pub fn new(
		pmmr: RewindablePMMR<'a, TxKernel, PMMRBackend<TxKernel>>,
		header: BlockHeader,
	) -> RewindableKernelView<'a> {
		RewindableKernelView { pmmr, header }
	}

	/// Rewind this readonly view to a previous block.
	/// We accomplish this in a readonly way because we can rewind the PMMR
	/// via last_pos, without rewinding the underlying backend files.
	pub fn rewind(&mut self, header: &BlockHeader) -> Result<(), Error> {
		self.pmmr
			.rewind(header.kernel_mmr_size)
			.map_err(&Error::TxHashSetErr)?;

		// Update our header to reflect the one we rewound to.
		self.header = header.clone();

		Ok(())
	}

	/// Special handling to make sure the whole kernel set matches each of its
	/// roots in each block header, without truncation. We go back header by
	/// header, rewind and check each root. This fixes a potential weakness in
	/// fast sync where a reorg past the horizon could allow a whole rewrite of
	/// the kernel set.
	pub fn validate_root(&self) -> Result<(), Error> {
		let root = self
			.readonly_pmmr()
			.root()
			.map_err(|_| Error::InvalidRoot)?;
		if root != self.header.kernel_root {
			return Err(Error::InvalidTxHashSet(format!(
				"Kernel root at {} does not match",
				self.header.height
			))
			.into());
		}
		Ok(())
	}

	/// Readonly view of our internal data.
	pub fn readonly_pmmr(&self) -> ReadonlyPMMR<TxKernel, PMMRBackend<TxKernel>> {
		self.pmmr.as_readonly()
	}
}
