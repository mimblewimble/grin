extern crate grin_core;
extern crate grin_keychain;

use std::path::Path;
use std::fs::{self, File};
use grin_core::ser;
use grin_core::core::{Block, BlockHeader, CompactBlock, Transaction};
use grin_core::core::target::Difficulty;

use grin_core::core::build::{input, output, transaction_with_offset, with_fee};
use grin_keychain::keychain::Keychain;

fn main() {
	generate("transaction_read", &tx()).unwrap();
	generate("block_read", &block()).unwrap();
	generate("compact_block_read", &compact_block()).unwrap();
}

fn generate<W: ser::Writeable>(target: &str, obj: W) -> Result<(), ser::Error> {
	let dir_path = Path::new("corpus").join(target);
	if !dir_path.is_dir() {
		fs::create_dir(&dir_path).map_err(|e| ser::Error::IOErr(e))?;
	}

	let pattern_path = dir_path.join("pattern");
	if !pattern_path.exists() {
		let mut file = File::create(&pattern_path).map_err(|e| ser::Error::IOErr(e))?;
		ser::serialize(&mut file, &obj)
	} else {
		Ok(())
	}
}

fn block() -> Block {
	let keychain = Keychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();

	let mut tx1 = tx();
	let mut tx2 = tx();

	Block::new(
		&BlockHeader::default(),
		vec![&mut tx1, &mut tx2],
		&keychain,
		&key_id,
		Difficulty::one(),
	).unwrap()
}

fn compact_block() -> CompactBlock {
	CompactBlock {
		header: BlockHeader::default(),
		nonce: 1,
		out_full: vec![],
		kern_full: vec![],
		kern_ids: vec![],
	}
}

fn tx() -> Transaction {
	let keychain = Keychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();

	transaction_with_offset(
		vec![
			input(10, key_id1),
			input(11, key_id2),
			output(19, key_id3),
			with_fee(2),
		],
		&keychain,
	).unwrap()
}
