use crate::core::core::hash::Hash;
use crate::core::core::{BlockHeader, OutputIdentifier, Segment, SegmentIdentifier, TxKernel};
use crate::error::Error;
use crate::txhashset::BitmapChunk;
use crate::util::secp::pedersen::RangeProof;

/// PLACEHOLDER
#[derive(Clone)]
pub struct Segmenter {}

impl Segmenter {
	/// PLACEHOLDER
	pub fn new() -> Segmenter {
		Segmenter {}
	}

	/// PLACEHOLDER
	pub fn header(&self) -> &BlockHeader {
		unimplemented!()
	}

	/// PLACEHOLDER
	pub fn kernel_segment(&self, _id: SegmentIdentifier) -> Result<Segment<TxKernel>, Error> {
		unimplemented!()
	}

	/// PLACEHOLDER
	pub fn bitmap_segment(
		&self,
		_id: SegmentIdentifier,
	) -> Result<(Segment<BitmapChunk>, Hash), Error> {
		unimplemented!()
	}

	/// PLACEHOLDER
	pub fn output_segment(
		&self,
		_id: SegmentIdentifier,
	) -> Result<(Segment<OutputIdentifier>, Hash), Error> {
		unimplemented!()
	}

	/// PLACEHOLDER
	pub fn rangeproof_segment(&self, _id: SegmentIdentifier) -> Result<Segment<RangeProof>, Error> {
		unimplemented!()
	}
}
