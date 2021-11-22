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

//! Manages the reconsitution of a txhashset from segments produced by the
//! segmenter

use std::{sync::Arc, time::Instant};

use crate::core::core::hash::Hash;
use crate::core::core::pmmr::ReadablePMMR;
use crate::core::core::{BlockHeader, OutputIdentifier, Segment, SegmentIdentifier, TxKernel};
use crate::error::{Error, ErrorKind};
use crate::txhashset::{BitmapAccumulator, BitmapChunk, TxHashSet};
use crate::util::secp::pedersen::RangeProof;
use crate::util::RwLock;

use crate::store;
use crate::txhashset;

/// Desegmenter for rebuilding a txhashset from PIBD segments
#[derive(Clone)]
pub struct Desegmenter {
	txhashset: Arc<RwLock<TxHashSet>>,
	header_pmmr: Arc<RwLock<txhashset::PMMRHandle<BlockHeader>>>,
	store: Arc<store::ChainStore>,

	bitmap_snapshot: Arc<BitmapAccumulator>,
	bitmap_segments: Vec<Segment<BitmapChunk>>,
	output_segments: Vec<Segment<OutputIdentifier>>,
	rangeproof_segments: Vec<Segment<RangeProof>>,
	kernel_segments: Vec<Segment<TxKernel>>,
}

impl Desegmenter {
	/// Create a new segmenter based on the provided txhashset.
	pub fn new(
		txhashset: Arc<RwLock<TxHashSet>>,
		header_pmmr: Arc<RwLock<txhashset::PMMRHandle<BlockHeader>>>,
		store: Arc<store::ChainStore>,
	) -> Desegmenter {
		Desegmenter {
			txhashset,
			header_pmmr,
			store,
			bitmap_snapshot: Arc::new(BitmapAccumulator::new()),
			bitmap_segments: vec![],
			output_segments: vec![],
			rangeproof_segments: vec![],
			kernel_segments: vec![],
		}
	}

	/// Adds a bitmap segment
	pub fn add_bitmap_segment(&mut self, segment: Segment<BitmapChunk>) -> Result<(), Error> {
		debug!("pibd_desegmenter write: add bitmap segment");
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let mut batch = self.store.batch()?;
		txhashset::extending(
			&mut header_pmmr,
			&mut txhashset,
			&mut batch,
			|ext, batch| {
				let extension = &mut ext.extension;

				Ok(())
			},
		)?;
		Ok(())
	}
}
