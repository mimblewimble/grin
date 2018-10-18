// Copyright 2018 The Grin Developers
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
use keychain::{Identifier, Keychain};

use core::consensus::reward;
use core::core::KernelFeatures;
use core::core::{Output, OutputFeatures, TxKernel};
use libtx::error::Error;
use libtx::{aggsig, proof};
use util::{kernel_sig_msg, secp, static_secp_instance};

/// output a reward output
pub fn output<K>(
	keychain: &K,
	key_id: &Identifier,
	fees: u64,
	height: u64,
) -> Result<(Output, TxKernel), Error>
where
	K: Keychain,
{
	let value = reward(fees);
	let commit = keychain.commit(value, key_id)?;

	trace!("Block reward - Pedersen Commit is: {:?}", commit,);

	let rproof = proof::create(keychain, value, key_id, commit, None)?;

	let output = Output {
		features: OutputFeatures::COINBASE_OUTPUT,
		commit: commit,
		proof: rproof,
	};

	let secp = static_secp_instance();
	let secp = secp.lock();
	let over_commit = secp.commit_value(reward(fees))?;
	let out_commit = output.commitment();
	let excess = secp.commit_sum(vec![out_commit], vec![over_commit])?;
	let pubkey = excess.to_pubkey(&secp)?;

	// NOTE: Remember we sign the fee *and* the lock_height.
	// For a coinbase output the fee is 0 and the lock_height is
	// the lock_height of the coinbase output itself,
	// not the lock_height of the tx (there is no tx for a coinbase output).
	// This output will not be spendable earlier than lock_height (and we sign this
	// here).
	let msg = secp::Message::from_slice(&kernel_sig_msg(0, height))?;
	let sig = aggsig::sign_from_key_id(&secp, keychain, &msg, &key_id, Some(&pubkey))?;

	let proof = TxKernel {
		features: KernelFeatures::COINBASE_KERNEL,
		excess: excess,
		excess_sig: sig,
		fee: 0,
		// lock_height here is the height of the block (tx should be valid immediately)
		// *not* the lock_height of the coinbase output (only spendable 1,000 blocks later)
		lock_height: height,
	};
	Ok((output, proof))
}
