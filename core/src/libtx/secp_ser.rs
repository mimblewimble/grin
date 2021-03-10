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

//! Sane serialization & deserialization of cryptographic structs into hex

use keychain::BlindingFactor;
use serde::{Deserialize, Deserializer, Serializer};
use util::secp::pedersen::{Commitment, RangeProof};
use util::{from_hex, ToHex};

/// Serializes a secp PublicKey to and from hex
pub mod pubkey_serde {
	use serde::{Deserialize, Deserializer, Serializer};
	use util::secp::key::PublicKey;
	use util::{from_hex, static_secp_instance, ToHex};

	///
	pub fn serialize<S>(key: &PublicKey, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();
		serializer.serialize_str(&key.serialize_vec(&static_secp, true).to_hex())
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
			.and_then(|string| from_hex(&string).map_err(Error::custom))
			.and_then(|bytes: Vec<u8>| {
				PublicKey::from_slice(&static_secp, &bytes).map_err(Error::custom)
			})
	}
}

/// Serializes an Option<secp::Signature> to and from hex
pub mod option_sig_serde {
	use serde::de::Error;
	use serde::{Deserialize, Deserializer, Serializer};
	use util::{from_hex, secp, static_secp_instance, ToHex};

	///
	pub fn serialize<S>(sig: &Option<secp::Signature>, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();
		match sig {
			Some(sig) => {
				serializer.serialize_str(&(&sig.serialize_compact(&static_secp)[..]).to_hex())
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
		Option::<String>::deserialize(deserializer).and_then(|res| match res {
			Some(string) => from_hex(&string)
				.map_err(Error::custom)
				.and_then(|bytes: Vec<u8>| {
					if bytes.len() < 64 {
						return Err(Error::invalid_length(bytes.len(), &"64 bytes"));
					}
					let mut b = [0u8; 64];
					b.copy_from_slice(&bytes[0..64]);
					secp::Signature::from_compact(&static_secp, &b)
						.map(Some)
						.map_err(Error::custom)
				}),
			None => Ok(None),
		})
	}
}

/// Serializes an Option<secp::SecretKey> to and from hex
pub mod option_seckey_serde {
	use serde::de::Error;
	use serde::{Deserialize, Deserializer, Serializer};
	use util::{from_hex, secp, static_secp_instance, ToHex};

	///
	pub fn serialize<S>(
		key: &Option<secp::key::SecretKey>,
		serializer: S,
	) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match key {
			Some(key) => serializer.serialize_str(&key.0.to_hex()),
			None => serializer.serialize_none(),
		}
	}

	///
	pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<secp::key::SecretKey>, D::Error>
	where
		D: Deserializer<'de>,
	{
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();
		Option::<String>::deserialize(deserializer).and_then(|res| match res {
			Some(string) => from_hex(&string)
				.map_err(Error::custom)
				.and_then(|bytes: Vec<u8>| {
					if bytes.len() < 32 {
						return Err(Error::invalid_length(bytes.len(), &"32 bytes"));
					}
					let mut b = [0u8; 32];
					b.copy_from_slice(&bytes[0..32]);
					secp::key::SecretKey::from_slice(&static_secp, &b)
						.map(Some)
						.map_err(Error::custom)
				}),
			None => Ok(None),
		})
	}
}

/// Serializes a secp::Signature to and from hex
pub mod sig_serde {
	use serde::de::Error;
	use serde::{Deserialize, Deserializer, Serializer};
	use util::{from_hex, secp, static_secp_instance, ToHex};

	///
	pub fn serialize<S>(sig: &secp::Signature, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();
		serializer.serialize_str(&(&sig.serialize_compact(&static_secp)[..]).to_hex())
	}

	///
	pub fn deserialize<'de, D>(deserializer: D) -> Result<secp::Signature, D::Error>
	where
		D: Deserializer<'de>,
	{
		let static_secp = static_secp_instance();
		let static_secp = static_secp.lock();
		String::deserialize(deserializer)
			.and_then(|string| from_hex(&string).map_err(Error::custom))
			.and_then(|bytes: Vec<u8>| {
				if bytes.len() < 64 {
					return Err(Error::invalid_length(bytes.len(), &"64 bytes"));
				}
				let mut b = [0u8; 64];
				b.copy_from_slice(&bytes[0..64]);
				secp::Signature::from_compact(&static_secp, &b).map_err(Error::custom)
			})
	}
}

/// Serializes an Option<secp::Commitment> to and from hex
pub mod option_commitment_serde {
	use serde::de::Error;
	use serde::{Deserialize, Deserializer, Serializer};
	use util::secp::pedersen::Commitment;
	use util::{from_hex, ToHex};

	///
	pub fn serialize<S>(commit: &Option<Commitment>, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match commit {
			Some(c) => serializer.serialize_str(&c.to_hex()),
			None => serializer.serialize_none(),
		}
	}

	///
	pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Commitment>, D::Error>
	where
		D: Deserializer<'de>,
	{
		Option::<String>::deserialize(deserializer).and_then(|res| match res {
			Some(string) => from_hex(&string)
				.map_err(Error::custom)
				.and_then(|bytes: Vec<u8>| Ok(Some(Commitment::from_vec(bytes.to_vec())))),
			None => Ok(None),
		})
	}
}
/// Creates a BlindingFactor from a hex string
pub fn blind_from_hex<'de, D>(deserializer: D) -> Result<BlindingFactor, D::Error>
where
	D: Deserializer<'de>,
{
	use serde::de::Error;
	String::deserialize(deserializer)
		.and_then(|string| BlindingFactor::from_hex(&string).map_err(Error::custom))
}

/// Creates a RangeProof from a hex string
pub fn rangeproof_from_hex<'de, D>(deserializer: D) -> Result<RangeProof, D::Error>
where
	D: Deserializer<'de>,
{
	use serde::de::{Error, IntoDeserializer};

	let val = String::deserialize(deserializer)
		.and_then(|string| from_hex(&string).map_err(Error::custom))?;
	RangeProof::deserialize(val.into_deserializer())
}

/// Creates a Pedersen Commitment from a hex string
pub fn commitment_from_hex<'de, D>(deserializer: D) -> Result<Commitment, D::Error>
where
	D: Deserializer<'de>,
{
	use serde::de::Error;
	String::deserialize(deserializer)
		.and_then(|string| from_hex(&string).map_err(Error::custom))
		.and_then(|bytes: Vec<u8>| Ok(Commitment::from_vec(bytes.to_vec())))
}

/// Seralizes a byte string into hex
pub fn as_hex<T, S>(bytes: T, serializer: S) -> Result<S::Ok, S::Error>
where
	T: AsRef<[u8]>,
	S: Serializer,
{
	serializer.serialize_str(&bytes.to_hex())
}

/// Used to ensure u64s are serialised in json
/// as strings by default, since it can't be guaranteed that consumers
/// will know what to do with u64 literals (e.g. Javascript). However,
/// fields using this tag can be deserialized from literals or strings.
/// From solutions on:
/// https://github.com/serde-rs/json/issues/329
pub mod string_or_u64 {
	use std::fmt;

	use serde::{de, Deserializer, Serializer};

	/// serialize into a string
	pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
	where
		T: fmt::Display,
		S: Serializer,
	{
		serializer.collect_str(value)
	}

	/// deserialize from either literal or string
	pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct Visitor;
		impl<'a> de::Visitor<'a> for Visitor {
			type Value = u64;
			fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
				write!(
					formatter,
					"a string containing digits or an int fitting into u64"
				)
			}
			fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
				Ok(v)
			}
			fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
			where
				E: de::Error,
			{
				s.parse().map_err(de::Error::custom)
			}
		}
		deserializer.deserialize_any(Visitor)
	}
}

/// As above, for Options
pub mod opt_string_or_u64 {
	use std::fmt;

	use serde::{de, Deserializer, Serializer};

	/// serialize into string or none
	pub fn serialize<T, S>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
	where
		T: fmt::Display,
		S: Serializer,
	{
		match value {
			Some(v) => serializer.collect_str(v),
			None => serializer.serialize_none(),
		}
	}

	/// deser from 'null', literal or string
	pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct Visitor;
		impl<'a> de::Visitor<'a> for Visitor {
			type Value = Option<u64>;
			fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
				write!(
					formatter,
					"null, a string containing digits or an int fitting into u64"
				)
			}
			fn visit_unit<E>(self) -> Result<Self::Value, E> {
				Ok(None)
			}
			fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
				Ok(Some(v))
			}
			fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
			where
				E: de::Error,
			{
				let val: u64 = s.parse().map_err(de::Error::custom)?;
				Ok(Some(val))
			}
		}
		deserializer.deserialize_any(Visitor)
	}
}

// Test serialization methods of components that are being used
#[cfg(test)]
mod test {
	use super::*;
	use crate::libtx::aggsig;
	use util::secp::key::{PublicKey, SecretKey};
	use util::secp::{Message, Signature};
	use util::static_secp_instance;

	use serde_json;

	use rand::{thread_rng, Rng};

	#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
	struct SerTest {
		#[serde(with = "option_seckey_serde")]
		pub opt_skey: Option<SecretKey>,
		#[serde(with = "pubkey_serde")]
		pub pub_key: PublicKey,
		#[serde(with = "option_sig_serde")]
		pub opt_sig: Option<Signature>,
		#[serde(with = "option_commitment_serde")]
		pub opt_commit: Option<Commitment>,
		#[serde(with = "sig_serde")]
		pub sig: Signature,
		#[serde(with = "string_or_u64")]
		pub num: u64,
		#[serde(with = "opt_string_or_u64")]
		pub opt_num: Option<u64>,
	}

	impl SerTest {
		pub fn random() -> SerTest {
			let static_secp = static_secp_instance();
			let secp = static_secp.lock();
			let sk = SecretKey::new(&secp, &mut thread_rng());
			let mut msg = [0u8; 32];
			thread_rng().fill(&mut msg);
			let msg = Message::from_slice(&msg).unwrap();
			let sig = aggsig::sign_single(&secp, &msg, &sk, None, None).unwrap();
			let mut commit = [0u8; 33];
			commit[0] = 0x09;
			thread_rng().fill(&mut commit[1..]);
			let commit = Commitment::from_vec(commit.to_vec());
			SerTest {
				opt_skey: Some(sk.clone()),
				pub_key: PublicKey::from_secret_key(&secp, &sk).unwrap(),
				opt_sig: Some(sig),
				opt_commit: Some(commit),
				sig: sig,
				num: 30,
				opt_num: Some(33),
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
