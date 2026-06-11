use self::chain::txhashset::{BitmapAccumulator, BitmapSegment};
use self::core::core::pmmr::segment::{Segment, SegmentIdentifier};
use self::core::ser::{
	self, BinReader, BinWriter, DeserializationMode, ProtocolVersion, Readable, Writeable,
};
use croaring::Bitmap;
use grin_chain as chain;
use grin_core as core;
use grin_util::secp::rand::Rng;
use rand::thread_rng;
use std::io::Cursor;

fn push_u16(bytes: &mut Vec<u8>, n: u16) {
	bytes.extend_from_slice(&n.to_be_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, n: u64) {
	bytes.extend_from_slice(&n.to_be_bytes());
}

fn bitmap_segment_header(height: u8, idx: u64, n_blocks: u16) -> Vec<u8> {
	let mut bytes = vec![height];
	push_u64(&mut bytes, idx);
	push_u16(&mut bytes, n_blocks);
	bytes
}

fn read_bitmap_segment(bytes: &[u8]) -> Result<BitmapSegment, ser::Error> {
	ser::deserialize(
		&mut &bytes[..],
		ProtocolVersion(1),
		DeserializationMode::default(),
	)
}

fn test_roundtrip(entries: usize) {
	let mut rng = thread_rng();

	let identifier = SegmentIdentifier {
		height: 12,
		idx: rng.gen_range(8, 16),
	};
	let block = rng.gen_range(2, 64);

	let mut bitmap = Bitmap::new();
	let block_size = 1 << 16;
	let offset = (1 << identifier.height) * 1024 * identifier.idx + block_size * block;
	let mut count = 0;
	while count < entries {
		let idx = (offset + rng.gen_range(0, block_size)) as u32;
		if !bitmap.contains(idx) {
			count += 1;
			bitmap.add(idx);
		}
	}

	// Add a bunch of segments after the one we are interested in
	let size =
		bitmap.maximum().unwrap() as u64 + (1 << identifier.height) * 1024 * rng.gen_range(0, 64);

	// Construct the accumulator
	let mut accumulator = BitmapAccumulator::new();
	accumulator
		.init(bitmap.iter().map(|v| v as u64), size)
		.unwrap();

	let mmr = accumulator.readonly_pmmr();
	let segment = Segment::from_pmmr(identifier, &mmr, false).unwrap();

	// Convert to `BitmapSegment`
	let bms = BitmapSegment::from(segment.clone());

	// Serialize `BitmapSegment`
	let mut cursor = Cursor::new(Vec::<u8>::new());
	let mut writer = BinWriter::new(&mut cursor, ProtocolVersion(1));
	Writeable::write(&bms, &mut writer).unwrap();

	// Read `BitmapSegment`
	cursor.set_position(0);
	let mut reader = BinReader::new(
		&mut cursor,
		ProtocolVersion(1),
		DeserializationMode::default(),
	);
	let bms2: BitmapSegment = Readable::read(&mut reader).unwrap();
	assert_eq!(bms, bms2);

	// Convert back to `Segment`
	let segment2 = bms2.into_segment().unwrap();
	assert_eq!(segment, segment2);
}

#[test]
fn segment_ser_roundtrip() {
	let threshold = 4096;
	test_roundtrip(thread_rng().gen_range(threshold, 4 * threshold));
}

#[test]
fn sparse_segment_ser_roundtrip() {
	test_roundtrip(thread_rng().gen_range(1024, 4096));
}

#[test]
fn abundant_segment_ser_roundtrip() {
	let max = 1 << 16;
	test_roundtrip(thread_rng().gen_range(max - 4096, max - 1024));
}

#[test]
fn bitmap_segment_read_rejects_empty_blocks() {
	let bytes = bitmap_segment_header(9, 0, 0);
	assert_eq!(
		read_bitmap_segment(&bytes).err(),
		Some(ser::Error::CorruptedData)
	);
}

#[test]
fn bitmap_segment_read_rejects_too_many_blocks() {
	let bytes = bitmap_segment_header(9, 0, 9);
	assert_eq!(
		read_bitmap_segment(&bytes).err(),
		Some(ser::Error::TooLargeReadErr)
	);
}

#[test]
fn bitmap_segment_read_rejects_too_large_height() {
	let bytes = bitmap_segment_header(14, 0, 1);
	assert_eq!(
		read_bitmap_segment(&bytes).err(),
		Some(ser::Error::TooLargeReadErr)
	);
}

#[test]
fn bitmap_segment_read_rejects_offset_overflow() {
	let bytes = bitmap_segment_header(13, u64::MAX, 1);
	assert_eq!(
		read_bitmap_segment(&bytes).err(),
		Some(ser::Error::TooLargeReadErr)
	);
}
