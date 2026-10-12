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
	#[allow(dead_code)]
	IoErr(std::io::Error),
	#[allow(dead_code)]
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
	let inputs: Vec<u64> = CORPUS_INPUT_VALUES.to_vec();
	let outputs: Vec<u64> = vec![5, 50, 500, 5000, 50000, 150000, 250000, 450000];
	let fee: u32 = inputs
		.iter()
		.sum::<u64>()
		.checked_sub(outputs.iter().sum())
		.expect("corpus inputs/outputs must yield a valid fee")
		.try_into()
		.expect("corpus fee must fit in FeeFields");
	let mut txes: Vec<FuzzTx> = vec![];

	// create valid txes of all supported types
	if let Some(tx) = fuzzer.test_transaction_spending_coinbase(outputs.clone()) {
		txes.push(FuzzTx {
			version: ser::ProtocolVersion::local().value(),
			name: "coinbase".into(),
			tx,
		});
	} else {
		eprintln!("WARN: skipping coinbase corpus seed with invalid fee");
	}
	if let Some(tx) = fuzzer.test_transaction(inputs.clone(), outputs.clone()) {
		txes.push(FuzzTx {
			version: ser::ProtocolVersion::local().value(),
			name: "plain".into(),
			tx,
		});
	} else {
		eprintln!("WARN: skipping plain corpus seed with invalid fee");
	}
	txes.push(FuzzTx {
		version: ser::ProtocolVersion::local().value(),
		name: "height-locked".into(),
		tx: fuzzer.test_transaction_with_kernel_features(
			inputs.clone(),
			outputs.clone(),
			KernelFeatures::HeightLocked {
				fee: fee.into(),
				lock_height: 4u64,
			},
		),
	});
	txes.push(FuzzTx {
		version: ser::ProtocolVersion::local().value(),
		name: "no-recent-duplicate".into(),
		tx: fuzzer.test_transaction_with_kernel_features(
			inputs.clone(),
			outputs.clone(),
			KernelFeatures::NoRecentDuplicate {
				fee: fee.into(),
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
	global::set_local_accept_fee_base(1);
	global::set_local_nrd_enabled(true);

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
		let mut reader = data;
		let tx: Result<Transaction, ser::Error> = ser::deserialize(
			&mut reader,
			ser::ProtocolVersion::local(),
			ser::DeserializationMode::default(),
		);
		// we only care about inputs that pass
		if tx.is_ok() {
			// attempt to add fuzzed tx to the transaction pool
			//   fuzz tx source on random first byte of fuzzer input
			//   add to tx pool, then stem pool
			match fuzzer
				.pool
				.add_to_pool(fuzz_tx_source(data[0]), tx.unwrap(), i, &header)
			{
				Ok(_) if i => assert!(fuzzer.pool.stempool.size() >= 1),
				Ok(_) => assert!(fuzzer.pool.total_size() >= 1),
				Err(_) => continue,
			}
		}
	}
});
