#![no_main]
use libfuzzer_sys::fuzz_target;

use std::fs::{self, File};
use std::io::BufWriter;

use grin_core::{
	core::{KernelFeatures, NRDRelativeHeight, Transaction},
	global, ser,
};

mod common;

use common::*;

#[derive(Debug)]
enum Error {
	IoErr(std::io::Error),
	SerErr(ser::Error),
}

struct FuzzTx {
	version: u32,
	name: String,
	tx: Transaction,
}

fn gen_tx_corpus() -> Result<(), Error> {
	let fuzzer = PoolFuzzer::new("fuzz/target/.transaction_pool_corpus");

	// create arbitrary inputs and outputs
	let inputs: Vec<u64> = vec![10, 100, 1000, 10000, 100000, 200000, 400000, 800000];
	let outputs: Vec<u64> = vec![5, 50, 500, 5000, 50000, 100000, 200000, 400000];
	let mut txes: Vec<FuzzTx> = vec![];

	// create valid txes of all supported types
	txes.push(FuzzTx {
		version: 1u32,
		name: "coinbase".into(),
		tx: fuzzer.test_transaction_spending_coinbase(outputs.clone()),
	});
	txes.push(FuzzTx {
		version: 1u32,
		name: "plain".into(),
		tx: fuzzer.test_transaction(inputs.clone(), outputs.clone()),
	});
	txes.push(FuzzTx {
		version: 2u32,
		name: "height-locked".into(),
		tx: fuzzer.test_transaction_with_kernel_features(
			inputs.clone(),
			outputs.clone(),
			KernelFeatures::HeightLocked {
				fee: 100u64,
				lock_height: 42u64,
			},
		),
	});
	txes.push(FuzzTx {
		version: 2u32,
		name: "no-recent-duplicate".into(),
		tx: fuzzer.test_transaction_with_kernel_features(
			inputs.clone(),
			outputs.clone(),
			KernelFeatures::NoRecentDuplicate {
				fee: 100u64,
				relative_height: NRDRelativeHeight::new(42u64).unwrap(),
			},
		),
	});

	fs::create_dir_all("fuzz/corpus/transaction_pool").map_err(|e| Error::IoErr(e))?;

	// write txes to corpus files
	for tx in txes {
		let dict = File::create(format!("fuzz/corpus/transaction_pool/{}", tx.name))
			.map_err(|e| Error::IoErr(e))?;
		let mut writer = BufWriter::new(dict);
		ser::serialize(&mut writer, ser::ProtocolVersion(tx.version), &tx.tx)
			.map_err(|e| Error::SerErr(e))?;
	}

	Ok(())
}

fuzz_target!(|data: &[u8]| {
	// skip if input is too short
	if data.len() < 80 {
		return ();
	}

	grin_util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);

	// check for corpus generation arguments
	// only generate corpus once, skipping on every other run
	if let Ok(gen_corpus) = std::env::var("GRIN_POOL_GEN_CORPUS") {
		if gen_corpus == "0" {
			gen_tx_corpus().unwrap();
			std::env::set_var("GRIN_POOL_GEN_CORPUS", "1");
		}
	}

	let mut fuzzer = PoolFuzzer::new("fuzz/target/.transaction_pool");

	let header = fuzzer.chain.head_header().unwrap();

	for &i in [true, false].iter() {
		// deserialize tx from fuzzer data
		let tx: Result<Transaction, ser::Error> =
			ser::deserialize(&mut data.clone(), ser::ProtocolVersion(2));
		// we only care about inputs that pass
		if tx.is_ok() {
			// attempt to add fuzzed tx to the transaction pool
			//   fuzz tx source on random first byte of fuzzer input
			//   add to tx pool, then stem pool
			match fuzzer
				.pool
				.add_to_pool(fuzz_tx_source(data[0]), tx.unwrap(), i, &header)
			{
				Ok(_) => assert!(fuzzer.pool.total_size() >= 1),
				Err(_) => continue,
			}
		}
	}
});
