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

//! Sane serialization & deserialization of cryptographic structs into hex

use crate::keychain::BlindingFactor;
use crate::serde::{Deserialize, Deserializer, Serializer};
use crate::util::secp::pedersen::{Commitment, RangeProof};
use crate::util::{from_hex, to_hex};

/// Serializes a secp PublicKey to and from hex
pub mod pubkey_serde {
	use crate::serde::{Deserialize, Deserializer, Serializer};
	use crate::util::secp::key::PublicKey;
	use crate::util::{from_hex, static_secp_instance, to_hex};

	///
	pub fn serialize<S>(key: &PublicKey, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();
		serializer.serialize_str(&to_hex(key.serialize_vec(&static_secp, false).to_vec()))
	}

	///
	pub fn deserialize<'de, D>(deserializer: D) -> Result<PublicKey, D::Error>
	where
		D: Deserializer<'de>,
	{
		use serde::de::Error;
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();
		String::deserialize(deserializer)
			.and_then(|string| from_hex(string).map_err(|err| Error::custom(err.to_string())))
			.and_then(|bytes: Vec<u8>| {
				PublicKey::from_slice(&static_secp, &bytes)
					.map_err(|err| Error::custom(err.to_string()))
			})
	}
}

/// Serializes an Option<secp::Signature> to and from hex
pub mod option_sig_serde {
	use crate::serde::{Deserialize, Deserializer, Serializer};
	use crate::util::secp;
	use crate::util::static_secp_instance;
	use crate::util::{from_hex, to_hex};
	use serde::de::Error;

	///
	pub fn serialize<S>(sig: &Option<secp::Signature>, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match sig {
			Some(sig) => {
				let static_secp = static_secp_instance();
				let static_secp = static_secp.lock();
				serializer.serialize_str(&to_hex(sig.serialize_der(&static_secp)))
			}
			None => serializer.serialize_none(),
		}
	}

	///
	pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<secp::Signature>, D::Error>
	where
		D: Deserializer<'de>,
	{
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();

		Option::<&str>::deserialize(deserializer).and_then(|res| match res {
			Some(string) => from_hex(string.to_string())
				.map_err(|err| Error::custom(err.to_string()))
				.and_then(|bytes: Vec<u8>| {
					secp::Signature::from_der(&static_secp, &bytes)
						.map(|val| Some(val))
						.map_err(|err| Error::custom(err.to_string()))
				}),
			None => Ok(None),
		})
	}

}

/// Serializes a secp::Signature to and from hex
pub mod sig_serde {
	use crate::serde::{Deserialize, Deserializer, Serializer};
	use crate::util::secp;
	use crate::util::static_secp_instance;
	use crate::util::{from_hex, to_hex};
	use serde::de::Error;

	///
	pub fn serialize<S>(sig: &secp::Signature, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();
		serializer.serialize_str(&to_hex(sig.serialize_der(&static_secp)))
	}

	///
	pub fn deserialize<'de, D>(deserializer: D) -> Result<secp::Signature, D::Error>
	where
		D: Deserializer<'de>,
	{
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();
		String::deserialize(deserializer)
			.and_then(|string| from_hex(string).map_err(|err| Error::custom(err.to_string())))
			.and_then(|bytes: Vec<u8>| {
				secp::Signature::from_der(&static_secp, &bytes)
					.map_err(|err| Error::custom(err.to_string()))
			})
	}
}

/// Creates a BlindingFactor from a hex string
pub fn blind_from_hex<'de, D>(deserializer: D) -> Result<BlindingFactor, D::Error>
where
	D: Deserializer<'de>,
{
	use serde::de::Error;
	String::deserialize(deserializer).and_then(|string| {
		BlindingFactor::from_hex(&string).map_err(|err| Error::custom(err.to_string()))
	})
}

/// Creates a RangeProof from a hex string
pub fn rangeproof_from_hex<'de, D>(deserializer: D) -> Result<RangeProof, D::Error>
where
	D: Deserializer<'de>,
{
	use serde::de::{Error, IntoDeserializer};

	let val = String::deserialize(deserializer)
		.and_then(|string| from_hex(string).map_err(|err| Error::custom(err.to_string())))?;
	RangeProof::deserialize(val.into_deserializer())
}

/// Creates a Pedersen Commitment from a hex string
pub fn commitment_from_hex<'de, D>(deserializer: D) -> Result<Commitment, D::Error>
where
	D: Deserializer<'de>,
{
	use serde::de::Error;
	String::deserialize(deserializer)
		.and_then(|string| from_hex(string).map_err(|err| Error::custom(err.to_string())))
		.and_then(|bytes: Vec<u8>| Ok(Commitment::from_vec(bytes.to_vec())))
}

/// Seralizes a byte string into hex
pub fn as_hex<T, S>(bytes: T, serializer: S) -> Result<S::Ok, S::Error>
where
	T: AsRef<[u8]>,
	S: Serializer,
{
	serializer.serialize_str(&to_hex(bytes.as_ref().to_vec()))
}

// Test serialization methods of components that are being used
#[cfg(test)]
mod test {
	use super::*;
	use crate::libtx::aggsig;
	use crate::util::secp::key::{PublicKey, SecretKey};
	use crate::util::secp::{Message, Signature};
	use crate::util::static_secp_instance;

	use serde_json;

	use rand::{thread_rng, Rng};

	#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
	struct SerTest {
		#[serde(with = "pubkey_serde")]
		pub pub_key: PublicKey,
		#[serde(with = "option_sig_serde")]
		pub opt_sig: Option<Signature>,
		#[serde(with = "sig_serde")]
		pub sig: Signature,
	}

	impl SerTest {
		pub fn random() -> SerTest {
			let static_secp = static_secp_instance();
			let secp = static_secp.lock();
			let sk = SecretKey::new(&secp, &mut thread_rng());
			let mut msg = [0u8; 32];
			thread_rng().fill(&mut msg);
			let msg = Message::from_slice(&msg).unwrap();
			let sig = aggsig::sign_single(&secp, &msg, &sk, None).unwrap();
			SerTest {
				pub_key: PublicKey::from_secret_key(&secp, &sk).unwrap(),
				opt_sig: Some(sig.clone()),
				sig: sig.clone(),
			}
		}
	}

	#[test]
	fn ser_secp_primitives() {
		for _ in 0..10 {
			let s = SerTest::random();
			println!("Before Serialization: {:?}", s);
			let serialized = serde_json::to_string_pretty(&s).unwrap();
			println!("JSON: {}", serialized);
			let deserialized: SerTest = serde_json::from_str(&serialized).unwrap();
			println!("After Serialization: {:?}", deserialized);
			println!();
			assert_eq!(s, deserialized);
		}
	}
}
