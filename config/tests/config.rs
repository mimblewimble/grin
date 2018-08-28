#[macro_use]
extern crate pretty_assertions;
extern crate grin_config as config;

use config::GlobalConfig;

#[test]
fn file_config_equal_to_defaults() {
	let global_config_without_file = GlobalConfig::default();

	let global_config_with_file = GlobalConfig::new(Some("../grin.toml")).unwrap_or_else(|e| {
		panic!("Error parsing config file: {}", e);
	});

	assert_eq!(
		global_config_without_file.members,
		global_config_with_file.members
	);
}
