//use crate::chain_test_helper::ChainTestHelper;
//use crate::chain_test_helper;//::{mine_chain, clean_output_dir};
//use grin_chain as chain;
mod chain_test_helper;
use self::chain_test_helper::{clean_output_dir, mine_chain};

#[test]
fn test() {
	let chain = mine_chain(".txhashset_archive_test", 25);
	let header = chain.txhashset_archive_header().unwrap();
	assert_eq!(10, header.height);
	clean_output_dir(".txhashset_archive_test");
}
