use self::chain::txhashset::{BitmapAccumulator, BitmapSegment};
use self::core::core::pmmr::segment::{Segment, SegmentIdentifier};
use self::core::ser::{
	BinReader, BinWriter, DeserializationMode, ProtocolVersion, Readable, Writeable,
};
use croaring::Bitmap;
use grin_chain as chain;
use grin_core as core;
use grin_util::secp::rand::Rng;
use rand::thread_rng;
use std::io::Cursor;

fn test_roundtrip(entries: usize) {
	let mut rng = thread_rng();

	let identifier = SegmentIdentifier {
		height: 12,
		idx: rng.gen_range(8, 16),
	};
	let block = rng.gen_range(2, 64);

	let mut bitmap = Bitmap::create();
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
	let segment2 = Segment::from(bms2);
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
