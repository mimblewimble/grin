// Copyright 2018 The Grin Developers
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! libwallet specific tests
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

extern crate rand;
extern crate uuid;

use uuid::Uuid;
use util::{kernel_sig_msg, secp};
use util::secp::key::SecretKey;
use util::secp::pedersen::ProofMessage;
use keychain::{BlindSum, BlindingFactor, Keychain};
use wallet::libwallet::{aggsig, proof};

use rand::thread_rng;

#[test]
fn aggsig_sender_receiver_interaction() {
	let sender_keychain = Keychain::from_random_seed().unwrap();
	let receiver_keychain = Keychain::from_random_seed().unwrap();
	let mut sender_aggsig_cm = aggsig::ContextManager::new();
	let mut receiver_aggsig_cm = aggsig::ContextManager::new();

	// tx identifier for wallet interaction
	let tx_id = Uuid::new_v4();

	// Calculate the kernel excess here for convenience.
	// Normally this would happen during transaction building.
	let kernel_excess = {
		let skey1 = sender_keychain
			.derived_key(&sender_keychain.derive_key_id(1).unwrap())
			.unwrap();

		let skey2 = receiver_keychain
			.derived_key(&receiver_keychain.derive_key_id(1).unwrap())
			.unwrap();

		let keychain = Keychain::from_random_seed().unwrap();
		let blinding_factor = keychain
			.blind_sum(&BlindSum::new()
				.sub_blinding_factor(BlindingFactor::from_secret_key(skey1))
				.add_blinding_factor(BlindingFactor::from_secret_key(skey2)))
			.unwrap();

		keychain
			.secp()
			.commit(0, blinding_factor.secret_key(&keychain.secp()).unwrap())
			.unwrap()
	};

	// sender starts the tx interaction
	let (sender_pub_excess, sender_pub_nonce) = {
		let keychain = sender_keychain.clone();

		let skey = keychain
			.derived_key(&keychain.derive_key_id(1).unwrap())
			.unwrap();

		// dealing with an input here so we need to negate the blinding_factor
		// rather than use it as is
		let bs = BlindSum::new();
		let blinding_factor = keychain
			.blind_sum(&bs.sub_blinding_factor(BlindingFactor::from_secret_key(skey)))
			.unwrap();

		let blind = blinding_factor.secret_key(&keychain.secp()).unwrap();

		let cx = sender_aggsig_cm.create_context(&keychain.secp(), &tx_id, blind);
		cx.get_public_keys(&keychain.secp())
	};

	// receiver receives partial tx
	let (receiver_pub_excess, receiver_pub_nonce, sig_part) = {
		let keychain = receiver_keychain.clone();
		let key_id = keychain.derive_key_id(1).unwrap();

		// let blind = blind_sum.secret_key(&keychain.secp())?;
		let blind = keychain.derived_key(&key_id).unwrap();

		let mut cx = receiver_aggsig_cm.create_context(&keychain.secp(), &tx_id, blind);
		let (pub_excess, pub_nonce) = cx.get_public_keys(&keychain.secp());
		cx.add_output(&key_id);

		let sig_part = cx.calculate_partial_sig(&keychain.secp(), &sender_pub_nonce, 0, 0)
			.unwrap();
		receiver_aggsig_cm.save_context(cx);
		(pub_excess, pub_nonce, sig_part)
	};

	// check the sender can verify the partial signature
	// received in the response back from the receiver
	{
		let keychain = sender_keychain.clone();
		let cx = sender_aggsig_cm.get_context(&tx_id);
		let sig_verifies = cx.verify_partial_sig(
			&keychain.secp(),
			&sig_part,
			&receiver_pub_nonce,
			&receiver_pub_excess,
			0,
			0,
		);
		assert!(sig_verifies);
	}

	// now sender signs with their key
	let sender_sig_part = {
		let keychain = sender_keychain.clone();
		let cx = sender_aggsig_cm.get_context(&tx_id);
		cx.calculate_partial_sig(&keychain.secp(), &receiver_pub_nonce, 0, 0)
			.unwrap()
	};

	// check the receiver can verify the partial signature
	// received by the sender
	{
		let keychain = receiver_keychain.clone();
		let cx = receiver_aggsig_cm.get_context(&tx_id);
		let sig_verifies = cx.verify_partial_sig(
			&keychain.secp(),
			&sender_sig_part,
			&sender_pub_nonce,
			&sender_pub_excess,
			0,
			0,
		);
		assert!(sig_verifies);
	}

	// Receiver now builds final signature from sender and receiver parts
	let (final_sig, final_pubkey) = {
		let keychain = receiver_keychain.clone();
		let cx = receiver_aggsig_cm.get_context(&tx_id);

		// Receiver recreates their partial sig (we do not maintain state from earlier)
		let our_sig_part = cx.calculate_partial_sig(&keychain.secp(), &sender_pub_nonce, 0, 0)
			.unwrap();

		// Receiver now generates final signature from the two parts
		let final_sig = cx.calculate_final_sig(
			&keychain.secp(),
			&sender_sig_part,
			&our_sig_part,
			&sender_pub_nonce,
		).unwrap();

		// Receiver calculates the final public key (to verify sig later)
		let final_pubkey = cx.calculate_final_pubkey(&keychain.secp(), &sender_pub_excess)
			.unwrap();

		(final_sig, final_pubkey)
	};

	// Receiver checks the final signature verifies
	{
		let keychain = receiver_keychain.clone();
		let cx = receiver_aggsig_cm.get_context(&tx_id);

		// Receiver check the final signature verifies
		let sig_verifies =
			cx.verify_final_sig_build_msg(&keychain.secp(), &final_sig, &final_pubkey, 0, 0);
		assert!(sig_verifies);
	}

	// Check we can verify the sig using the kernel excess
	{
		let keychain = Keychain::from_random_seed().unwrap();

		let msg = secp::Message::from_slice(&kernel_sig_msg(0, 0)).unwrap();

		let sig_verifies =
			aggsig::verify_single_from_commit(&keychain.secp(), &final_sig, &msg, &kernel_excess);

		assert!(sig_verifies);
	}
}

#[test]
fn aggsig_sender_receiver_interaction_offset() {
	let sender_keychain = Keychain::from_random_seed().unwrap();
	let receiver_keychain = Keychain::from_random_seed().unwrap();
	let mut sender_aggsig_cm = aggsig::ContextManager::new();
	let mut receiver_aggsig_cm = aggsig::ContextManager::new();

	// tx identifier for wallet interaction
	let tx_id = Uuid::new_v4();

	// This is the kernel offset that we use to split the key
	// Summing these at the block level prevents the
	// kernels from being used to reconstruct (or identify) individual transactions
	let kernel_offset = SecretKey::new(&sender_keychain.secp(), &mut thread_rng());

	// Calculate the kernel excess here for convenience.
	// Normally this would happen during transaction building.
	let kernel_excess = {
		let skey1 = sender_keychain
			.derived_key(&sender_keychain.derive_key_id(1).unwrap())
			.unwrap();

		let skey2 = receiver_keychain
			.derived_key(&receiver_keychain.derive_key_id(1).unwrap())
			.unwrap();

		let keychain = Keychain::from_random_seed().unwrap();
		let blinding_factor = keychain
			.blind_sum(&BlindSum::new()
				.sub_blinding_factor(BlindingFactor::from_secret_key(skey1))
				.add_blinding_factor(BlindingFactor::from_secret_key(skey2))
				// subtract the kernel offset here like as would when
				// verifying a kernel signature
				.sub_blinding_factor(BlindingFactor::from_secret_key(kernel_offset)))
			.unwrap();

		keychain
			.secp()
			.commit(0, blinding_factor.secret_key(&keychain.secp()).unwrap())
			.unwrap()
	};

	// sender starts the tx interaction
	let (sender_pub_excess, sender_pub_nonce) = {
		let keychain = sender_keychain.clone();

		let skey = keychain
			.derived_key(&keychain.derive_key_id(1).unwrap())
			.unwrap();

		// dealing with an input here so we need to negate the blinding_factor
		// rather than use it as is
		let blinding_factor = keychain
			.blind_sum(&BlindSum::new()
				.sub_blinding_factor(BlindingFactor::from_secret_key(skey))
				// subtract the kernel offset to create an aggsig context
				// with our "split" key
				.sub_blinding_factor(BlindingFactor::from_secret_key(kernel_offset)))
			.unwrap();

		let blind = blinding_factor.secret_key(&keychain.secp()).unwrap();

		let cx = sender_aggsig_cm.create_context(&keychain.secp(), &tx_id, blind);
		cx.get_public_keys(&keychain.secp())
	};

	// receiver receives partial tx
	let (receiver_pub_excess, receiver_pub_nonce, sig_part) = {
		let keychain = receiver_keychain.clone();
		let key_id = keychain.derive_key_id(1).unwrap();

		let blind = keychain.derived_key(&key_id).unwrap();

		let mut cx = receiver_aggsig_cm.create_context(&keychain.secp(), &tx_id, blind);
		let (pub_excess, pub_nonce) = cx.get_public_keys(&keychain.secp());
		cx.add_output(&key_id);

		let sig_part = cx.calculate_partial_sig(&keychain.secp(), &sender_pub_nonce, 0, 0)
			.unwrap();
		receiver_aggsig_cm.save_context(cx);
		(pub_excess, pub_nonce, sig_part)
	};

	// check the sender can verify the partial signature
	// received in the response back from the receiver
	{
		let keychain = sender_keychain.clone();
		let cx = sender_aggsig_cm.get_context(&tx_id);
		let sig_verifies = cx.verify_partial_sig(
			&keychain.secp(),
			&sig_part,
			&receiver_pub_nonce,
			&receiver_pub_excess,
			0,
			0,
		);
		assert!(sig_verifies);
	}

	// now sender signs with their key
	let sender_sig_part = {
		let keychain = sender_keychain.clone();
		let cx = sender_aggsig_cm.get_context(&tx_id);
		cx.calculate_partial_sig(&keychain.secp(), &receiver_pub_nonce, 0, 0)
			.unwrap()
	};

	// check the receiver can verify the partial signature
	// received by the sender
	{
		let keychain = receiver_keychain.clone();
		let cx = receiver_aggsig_cm.get_context(&tx_id);
		let sig_verifies = cx.verify_partial_sig(
			&keychain.secp(),
			&sender_sig_part,
			&sender_pub_nonce,
			&sender_pub_excess,
			0,
			0,
		);
		assert!(sig_verifies);
	}

	// Receiver now builds final signature from sender and receiver parts
	let (final_sig, final_pubkey) = {
		let keychain = receiver_keychain.clone();
		let cx = receiver_aggsig_cm.get_context(&tx_id);

		// Receiver recreates their partial sig (we do not maintain state from earlier)
		let our_sig_part = cx.calculate_partial_sig(&keychain.secp(), &sender_pub_nonce, 0, 0)
			.unwrap();

		// Receiver now generates final signature from the two parts
		let final_sig = cx.calculate_final_sig(
			&keychain.secp(),
			&sender_sig_part,
			&our_sig_part,
			&sender_pub_nonce,
		).unwrap();

		// Receiver calculates the final public key (to verify sig later)
		let final_pubkey = cx.calculate_final_pubkey(&keychain.secp(), &sender_pub_excess)
			.unwrap();

		(final_sig, final_pubkey)
	};

	// Receiver checks the final signature verifies
	{
		let keychain = receiver_keychain.clone();
		let cx = receiver_aggsig_cm.get_context(&tx_id);

		// Receiver check the final signature verifies
		let sig_verifies =
			cx.verify_final_sig_build_msg(&keychain.secp(), &final_sig, &final_pubkey, 0, 0);
		assert!(sig_verifies);
	}

	// Check we can verify the sig using the kernel excess
	{
		let keychain = Keychain::from_random_seed().unwrap();

		let msg = secp::Message::from_slice(&kernel_sig_msg(0, 0)).unwrap();

		let sig_verifies =
			aggsig::verify_single_from_commit(&keychain.secp(), &final_sig, &msg, &kernel_excess);

		assert!(sig_verifies);
	}
}

#[test]
fn test_rewind_range_proof() {
	let keychain = Keychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();
	let commit = keychain.commit(5, &key_id).unwrap();
	let msg = ProofMessage::from_bytes(&[0u8; 64]);
	let extra_data = [99u8; 64];

	let proof = proof::create(
		&keychain,
		5,
		&key_id,
		commit,
		Some(extra_data.to_vec().clone()),
		msg,
	).unwrap();
	let proof_info = proof::rewind(
		&keychain,
		&key_id,
		commit,
		Some(extra_data.to_vec().clone()),
		proof,
	).unwrap();

	assert_eq!(proof_info.success, true);

	// now check the recovered message is "empty" (but not truncated) i.e. all
	// zeroes
	//Value is in the message in this case
	assert_eq!(
		proof_info.message,
		secp::pedersen::ProofMessage::from_bytes(&[0; secp::constants::BULLET_PROOF_MSG_SIZE])
	);

	let key_id2 = keychain.derive_key_id(2).unwrap();

	// cannot rewind with a different nonce
	let proof_info = proof::rewind(
		&keychain,
		&key_id2,
		commit,
		Some(extra_data.to_vec().clone()),
		proof,
	).unwrap();
	// With bullet proofs, if you provide the wrong nonce you'll get gibberish back
	// as opposed to a failure to recover the message
	assert_ne!(
		proof_info.message,
		secp::pedersen::ProofMessage::from_bytes(&[0; secp::constants::BULLET_PROOF_MSG_SIZE])
	);
	assert_eq!(proof_info.value, 0);

	// cannot rewind with a commitment to the same value using a different key
	let commit2 = keychain.commit(5, &key_id2).unwrap();
	let proof_info = proof::rewind(
		&keychain,
		&key_id,
		commit2,
		Some(extra_data.to_vec().clone()),
		proof,
	).unwrap();
	assert_eq!(proof_info.success, false);
	assert_eq!(proof_info.value, 0);

	// cannot rewind with a commitment to a different value
	let commit3 = keychain.commit(4, &key_id).unwrap();
	let proof_info = proof::rewind(
		&keychain,
		&key_id,
		commit3,
		Some(extra_data.to_vec().clone()),
		proof,
	).unwrap();
	assert_eq!(proof_info.success, false);
	assert_eq!(proof_info.value, 0);

	// cannot rewind with wrong extra committed data
	let commit3 = keychain.commit(4, &key_id).unwrap();
	let wrong_extra_data = [98u8; 64];
	let _should_err = proof::rewind(
		&keychain,
		&key_id,
		commit3,
		Some(wrong_extra_data.to_vec().clone()),
		proof,
	).unwrap();

	assert_eq!(proof_info.success, false);
	assert_eq!(proof_info.value, 0);
}
