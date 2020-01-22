extern crate grin_core;
extern crate grin_keychain;

use grin_core::core::{Block, CompactBlock, Transaction};
use grin_core::ser;
use std::fs::{self, File};
use std::path::Path;

fn main() {
	generate(
		"transaction_read_v1",
		ser::ProtocolVersion(1),
		Transaction::default(),
	)
	.unwrap();
	generate("block_read_v1", ser::ProtocolVersion(1), Block::default()).unwrap();
	generate(
		"compact_block_read_v1",
		ser::ProtocolVersion(1),
		CompactBlock::from(Block::default()),
	)
	.unwrap();
	generate(
		"transaction_read_v2",
		ser::ProtocolVersion(2),
		Transaction::default(),
	)
	.unwrap();
	generate("block_read_v2", ser::ProtocolVersion(2), Block::default()).unwrap();
	generate(
		"compact_block_read_v2",
		ser::ProtocolVersion(2),
		CompactBlock::from(Block::default()),
	)
	.unwrap();
}

fn generate<W: ser::Writeable>(
	target: &str,
	version: ser::ProtocolVersion,
	obj: W,
) -> Result<(), ser::Error> {
	let dir_path = Path::new("corpus").join(target);
	if !dir_path.is_dir() {
		fs::create_dir_all(&dir_path).map_err(|e| {
			println!("fail: {}", e);
			ser::Error::IOErr("can't create corpus directory".to_owned(), e.kind())
		})?;
	}

	let pattern_path = dir_path.join("pattern");
	if !pattern_path.exists() {
		let mut file = File::create(&pattern_path)
			.map_err(|e| ser::Error::IOErr("can't create a pattern file".to_owned(), e.kind()))?;
		ser::serialize(&mut file, version, &obj)
	} else {
		Ok(())
	}
}
