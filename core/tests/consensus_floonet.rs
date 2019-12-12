use grin_core::consensus::{
	secondary_pow_ratio, valid_header_version, FLOONET_FIRST_HARD_FORK, FLOONET_SECOND_HARD_FORK,
	YEAR_HEIGHT,
};
use grin_core::core::HeaderVersion;
use grin_core::global;

#[test]
fn test_secondary_pow_ratio() {
	// Tests for Floonet chain type (covers pre and post hardfork).
	global::set_mining_mode(global::ChainTypes::Floonet);
	assert_eq!(global::is_floonet(), true);

	assert_eq!(secondary_pow_ratio(1), 90);
	assert_eq!(secondary_pow_ratio(89), 90);
	assert_eq!(secondary_pow_ratio(90), 90);
	assert_eq!(secondary_pow_ratio(91), 90);
	assert_eq!(secondary_pow_ratio(179), 90);
	assert_eq!(secondary_pow_ratio(180), 90);
	assert_eq!(secondary_pow_ratio(181), 90);

	let one_week = 60 * 24 * 7;
	assert_eq!(secondary_pow_ratio(one_week - 1), 90);
	assert_eq!(secondary_pow_ratio(one_week), 90);
	assert_eq!(secondary_pow_ratio(one_week + 1), 90);

	let two_weeks = one_week * 2;
	assert_eq!(secondary_pow_ratio(two_weeks - 1), 89);
	assert_eq!(secondary_pow_ratio(two_weeks), 89);
	assert_eq!(secondary_pow_ratio(two_weeks + 1), 89);

	let t4_fork_height = 64_000;
	assert_eq!(secondary_pow_ratio(t4_fork_height - 1), 85);
	assert_eq!(secondary_pow_ratio(t4_fork_height), 85);
	assert_eq!(secondary_pow_ratio(t4_fork_height + 1), 85);

	let one_year = one_week * 52;
	assert_eq!(secondary_pow_ratio(one_year), 45);

	let ninety_one_weeks = one_week * 91;
	assert_eq!(secondary_pow_ratio(ninety_one_weeks - 1), 12);
	assert_eq!(secondary_pow_ratio(ninety_one_weeks), 12);
	assert_eq!(secondary_pow_ratio(ninety_one_weeks + 1), 12);

	let two_year = one_year * 2;
	assert_eq!(secondary_pow_ratio(two_year - 1), 1);
	assert_eq!(secondary_pow_ratio(two_year), 0);
	assert_eq!(secondary_pow_ratio(two_year + 1), 0);
}

#[test]
fn hard_forks() {
	global::set_mining_mode(global::ChainTypes::Floonet);
	assert_eq!(global::is_floonet(), true);
	assert!(valid_header_version(0, HeaderVersion(1)));
	assert!(valid_header_version(10, HeaderVersion(1)));
	assert!(!valid_header_version(10, HeaderVersion(2)));
	assert!(valid_header_version(
		FLOONET_FIRST_HARD_FORK - 1,
		HeaderVersion(1)
	));
	assert!(valid_header_version(
		FLOONET_FIRST_HARD_FORK,
		HeaderVersion(2)
	));
	assert!(valid_header_version(
		FLOONET_FIRST_HARD_FORK + 1,
		HeaderVersion(2)
	));
	assert!(!valid_header_version(
		FLOONET_FIRST_HARD_FORK,
		HeaderVersion(1)
	));
	assert!(!valid_header_version(YEAR_HEIGHT, HeaderVersion(1)));
	assert!(valid_header_version(
		FLOONET_SECOND_HARD_FORK - 1,
		HeaderVersion(2)
	));
	assert!(valid_header_version(
		FLOONET_SECOND_HARD_FORK,
		HeaderVersion(3)
	));
	assert!(valid_header_version(
		FLOONET_SECOND_HARD_FORK + 1,
		HeaderVersion(3)
	));
	assert!(!valid_header_version(
		FLOONET_SECOND_HARD_FORK,
		HeaderVersion(2)
	));
	assert!(!valid_header_version(
		FLOONET_SECOND_HARD_FORK,
		HeaderVersion(1)
	));

	assert!(!valid_header_version(YEAR_HEIGHT - 1, HeaderVersion(2)));
	assert!(valid_header_version(YEAR_HEIGHT - 1, HeaderVersion(3)));
	assert!(valid_header_version(YEAR_HEIGHT, HeaderVersion(3)));
	assert!(valid_header_version(YEAR_HEIGHT + 1, HeaderVersion(3)));
	// v4 not active yet
	assert!(!valid_header_version(YEAR_HEIGHT * 3 / 2, HeaderVersion(4)));
	assert!(!valid_header_version(YEAR_HEIGHT * 3 / 2, HeaderVersion(3)));
	assert!(!valid_header_version(YEAR_HEIGHT * 3 / 2, HeaderVersion(2)));
	assert!(!valid_header_version(YEAR_HEIGHT * 3 / 2, HeaderVersion(1)));
	assert!(!valid_header_version(YEAR_HEIGHT * 2, HeaderVersion(3)));
	assert!(!valid_header_version(
		YEAR_HEIGHT * 3 / 2 + 1,
		HeaderVersion(3)
	));
}
