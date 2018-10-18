extern crate grin_core;
extern crate grin_keychain;
extern crate grin_wallet;

use grin_core::core::target::Difficulty;
use grin_core::core::{Block, BlockHeader, CompactBlock, Transaction};
use grin_core::ser;
use grin_keychain::keychain::ExtKeychain;
use grin_keychain::Keychain;
use grin_wallet::libtx::build::{input, output, transaction, with_fee};
use grin_wallet::libtx::reward;
use std::fs::{self, File};
use std::path::Path;

fn main() {
	generate("transaction_read", &tx()).unwrap();
	generate("block_read", &block()).unwrap();
	generate("compact_block_read", &compact_block()).unwrap();
}

fn generate<W: ser::Writeable>(target: &str, obj: W) -> Result<(), ser::Error> {
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
		ser::serialize(&mut file, &obj)
	} else {
		Ok(())
	}
}

fn block() -> Block {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();

	let mut txs = Vec::new();
	for _ in 1..10 {
		txs.push(tx());
	}

	let header = BlockHeader::default();

	let reward = reward::output(&keychain, &key_id, 0, header.height).unwrap();

	Block::new(&header, txs, Difficulty::min(), reward).unwrap()
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
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();

	transaction(
		vec![
			input(10, key_id1),
			input(11, key_id2),
			output(19, key_id3),
			with_fee(2),
		],
		&keychain,
	).unwrap()
}
