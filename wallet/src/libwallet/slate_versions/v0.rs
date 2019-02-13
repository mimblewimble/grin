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

// Contains V0 of the slate
use crate::core::core::transaction::{
	Input, KernelFeatures, Output, OutputFeatures, Transaction, TransactionBody, TxKernel,
};
use crate::keychain::BlindingFactor;
use crate::libwallet::slate::{ParticipantData, Slate};
use crate::util::secp;
use crate::util::secp::key::PublicKey;
use crate::util::secp::pedersen::{Commitment, RangeProof};
use crate::util::secp::Signature;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SlateV0 {
	/// The number of participants intended to take part in this transaction
	pub num_participants: usize,
	/// Unique transaction ID, selected by sender
	pub id: Uuid,
	/// The core transaction data:
	/// inputs, outputs, kernels, kernel offset
	pub tx: TransactionV0,
	/// base amount (excluding fee)
	pub amount: u64,
	/// fee amount
	pub fee: u64,
	/// Block height for the transaction
	pub height: u64,
	/// Lock height
	pub lock_height: u64,
	/// Participant data, each participant in the transaction will
	/// insert their public data here. For now, 0 is sender and 1
	/// is receiver, though this will change for multi-party
	pub participant_data: Vec<ParticipantDataV0>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParticipantDataV0 {
	/// Id of participant in the transaction. (For now, 0=sender, 1=rec)
	pub id: u64,
	/// Public key corresponding to private blinding factor
	pub public_blind_excess: PublicKey,
	/// Public key corresponding to private nonce
	pub public_nonce: PublicKey,
	/// Public partial signature
	pub part_sig: Option<Signature>,
	/// A message for other participants
	pub message: Option<String>,
	/// Signature, created with private key corresponding to 'public_blind_excess'
	pub message_sig: Option<Signature>,
}

/// A transaction
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransactionV0 {
	/// The kernel "offset" k2
	/// excess is k1G after splitting the key k = k1 + k2
	pub offset: BlindingFactor,
	/// The transaction body - inputs/outputs/kernels
	pub body: TransactionBodyV0,
}

/// TransactionBody is a common abstraction for transaction and block
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransactionBodyV0 {
	/// List of inputs spent by the transaction.
	pub inputs: Vec<InputV0>,
	/// List of outputs the transaction produces.
	pub outputs: Vec<OutputV0>,
	/// List of kernels that make up this transaction (usually a single kernel).
	pub kernels: Vec<TxKernelV0>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InputV0 {
	/// The features of the output being spent.
	/// We will check maturity for coinbase output.
	pub features: OutputFeatures,
	/// The commit referencing the output being spent.
	pub commit: Commitment,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct OutputV0 {
	/// Options for an output's structure or use
	pub features: OutputFeatures,
	/// The homomorphic commitment representing the output amount
	pub commit: Commitment,
	/// A proof that the commitment is in the right range
	pub proof: RangeProof,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TxKernelV0 {
	/// Options for a kernel's structure or use
	pub features: KernelFeatures,
	/// Fee originally included in the transaction this proof is for.
	pub fee: u64,
	/// This kernel is not valid earlier than lock_height blocks
	/// The max lock_height of all *inputs* to this transaction
	pub lock_height: u64,
	/// Remainder of the sum of all transaction commitments. If the transaction
	/// is well formed, amounts components should sum to zero and the excess
	/// is hence a valid public key.
	pub excess: Commitment,
	/// The signature proving the excess is a valid public key, which signs
	/// the transaction fee.
	pub excess_sig: secp::Signature,
}

impl From<SlateV0> for Slate {
	fn from(slate: SlateV0) -> Slate {
		let SlateV0 {
			num_participants,
			id,
			tx,
			amount,
			fee,
			height,
			lock_height,
			participant_data,
		} = slate;
		let tx = Transaction::from(tx);
		let participant_data = map_vec!(participant_data, |data| ParticipantData::from(data));
		let version = 0;
		Slate {
			num_participants,
			id,
			tx,
			amount,
			fee,
			height,
			lock_height,
			participant_data,
			version,
		}
	}
}

impl From<&ParticipantDataV0> for ParticipantData {
	fn from(data: &ParticipantDataV0) -> ParticipantData {
		let ParticipantDataV0 {
			id,
			public_blind_excess,
			public_nonce,
			part_sig,
			message,
			message_sig,
		} = data;
		let id = *id;
		let public_blind_excess = *public_blind_excess;
		let public_nonce = *public_nonce;
		let part_sig = *part_sig;
		let message: Option<String> = message.as_ref().map(|t| String::from(&**t));
		let message_sig = *message_sig;
		ParticipantData {
			id,
			public_blind_excess,
			public_nonce,
			part_sig,
			message,
			message_sig,
		}
	}
}

impl From<TransactionV0> for Transaction {
	fn from(tx: TransactionV0) -> Transaction {
		let TransactionV0 { offset, body } = tx;
		let body = TransactionBody::from(&body);
		let transaction = Transaction::new(body.inputs, body.outputs, body.kernels);
		transaction.with_offset(offset)
	}
}

impl From<&TransactionBodyV0> for TransactionBody {
	fn from(body: &TransactionBodyV0) -> Self {
		let TransactionBodyV0 {
			inputs,
			outputs,
			kernels,
		} = body;

		let inputs = map_vec!(inputs, |inp| Input::from(inp));
		let outputs = map_vec!(outputs, |out| Output::from(out));
		let kernels = map_vec!(kernels, |kern| TxKernel::from(kern));
		TransactionBody {
			inputs,
			outputs,
			kernels,
		}
	}
}

impl From<&InputV0> for Input {
	fn from(input: &InputV0) -> Input {
		let InputV0 { features, commit } = *input;
		Input { features, commit }
	}
}

impl From<&OutputV0> for Output {
	fn from(output: &OutputV0) -> Output {
		let OutputV0 {
			features,
			commit,
			proof,
		} = *output;
		Output {
			features,
			commit,
			proof,
		}
	}
}

impl From<&TxKernelV0> for TxKernel {
	fn from(kernel: &TxKernelV0) -> TxKernel {
		let TxKernelV0 {
			features,
			fee,
			lock_height,
			excess,
			excess_sig,
		} = *kernel;
		TxKernel {
			features,
			fee,
			lock_height,
			excess,
			excess_sig,
		}
	}
}

impl From<Slate> for SlateV0 {
	fn from(slate: Slate) -> SlateV0 {
		let Slate {
			num_participants,
			id,
			tx,
			amount,
			fee,
			height,
			lock_height,
			participant_data,
			version: _,
		} = slate;
		let tx = TransactionV0::from(tx);
		let participant_data = map_vec!(participant_data, |data| ParticipantDataV0::from(data));
		SlateV0 {
			num_participants,
			id,
			tx,
			amount,
			fee,
			height,
			lock_height,
			participant_data,
		}
	}
}

impl From<&ParticipantData> for ParticipantDataV0 {
	fn from(data: &ParticipantData) -> ParticipantDataV0 {
		let ParticipantData {
			id,
			public_blind_excess,
			public_nonce,
			part_sig,
			message,
			message_sig,
		} = data;
		let id = *id;
		let public_blind_excess = *public_blind_excess;
		let public_nonce = *public_nonce;
		let part_sig = *part_sig;
		let message: Option<String> = message.as_ref().map(|t| String::from(&**t));
		let message_sig = *message_sig;
		ParticipantDataV0 {
			id,
			public_blind_excess,
			public_nonce,
			part_sig,
			message,
			message_sig,
		}
	}
}

impl From<Transaction> for TransactionV0 {
	fn from(tx: Transaction) -> TransactionV0 {
		let offset = tx.offset;
		let body: TransactionBody = tx.into();
		let body = TransactionBodyV0::from(&body);
		TransactionV0 { offset, body }
	}
}

impl From<&TransactionBody> for TransactionBodyV0 {
	fn from(body: &TransactionBody) -> Self {
		let TransactionBody {
			inputs,
			outputs,
			kernels,
		} = body;

		let inputs = map_vec!(inputs, |inp| InputV0::from(inp));
		let outputs = map_vec!(outputs, |out| OutputV0::from(out));
		let kernels = map_vec!(kernels, |kern| TxKernelV0::from(kern));
		TransactionBodyV0 {
			inputs,
			outputs,
			kernels,
		}
	}
}

impl From<&Input> for InputV0 {
	fn from(input: &Input) -> Self {
		let Input { features, commit } = *input;
		InputV0 { features, commit }
	}
}

impl From<&Output> for OutputV0 {
	fn from(output: &Output) -> Self {
		let Output {
			features,
			commit,
			proof,
		} = *output;
		OutputV0 {
			features,
			commit,
			proof,
		}
	}
}

impl From<&TxKernel> for TxKernelV0 {
	fn from(kernel: &TxKernel) -> Self {
		let TxKernel {
			features,
			fee,
			lock_height,
			excess,
			excess_sig,
		} = *kernel;
		TxKernelV0 {
			features,
			fee,
			lock_height,
			excess,
			excess_sig,
		}
	}
}
