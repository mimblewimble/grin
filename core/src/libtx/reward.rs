// Copyright 2021 The Grin Developers
//
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

//! Builds the blinded output and related signature proof for the block
//! reward.
use crate::consensus::reward;
use crate::core::{KernelFeatures, Output, OutputFeatures, TxKernel};
use crate::libtx::error::Error;
use crate::libtx::{
	aggsig,
	proof::{self, ProofBuild},
};
use keychain::{Identifier, Keychain, SwitchCommitmentType};
use util::{secp, static_secp_instance};

/// output a reward output
pub fn output<K, B>(
	keychain: &K,
	builder: &B,
	key_id: &Identifier,
	fees: u64,
	test_mode: bool,
) -> Result<(Output, TxKernel), Error>
where
	K: Keychain,
	B: ProofBuild,
{
	let value = reward(fees);
	// TODO: proper support for different switch commitment schemes
	let switch = SwitchCommitmentType::Regular;
	let commit = keychain.commit(value, key_id, switch)?;

	trace!("Block reward - Pedersen Commit is: {:?}", commit,);

	let proof = proof::create(keychain, builder, value, key_id, switch, commit, None)?;

	let output = Output::new(OutputFeatures::Coinbase, commit, proof);

	let secp = static_secp_instance();
	let secp = secp.lock();
	let over_commit = secp.commit_value(reward(fees))?;
	let out_commit = output.commitment();
	let excess = secp.commit_sum(vec![out_commit], vec![over_commit])?;
	let pubkey = excess.to_pubkey(&secp)?;

	let features = KernelFeatures::Coinbase;
	let msg = features.kernel_sig_msg()?;
	let sig = match test_mode {
		true => {
			let test_nonce = secp::key::SecretKey::from_slice(&secp, &[1; 32])?;
			aggsig::sign_from_key_id(
				&secp,
				keychain,
				&msg,
				value,
				&key_id,
				Some(&test_nonce),
				Some(&pubkey),
			)?
		}
		false => {
			aggsig::sign_from_key_id(&secp, keychain, &msg, value, &key_id, None, Some(&pubkey))?
		}
	};

	let kernel = TxKernel {
		features: KernelFeatures::Coinbase,
		excess,
		excess_sig: sig,
	};
	Ok((output, kernel))
}
