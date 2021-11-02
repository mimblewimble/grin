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
//
//! # BIP39 Implementation
//!
//! Implementation of BIP39 Mnemonic code for generating deterministic keys, as defined
//! at https://github.com/bitcoin/bips/blob/master/bip-0039.mediawiki

use digest::Digest;
use hmac::Hmac;
use pbkdf2::pbkdf2;
use sha2::{Sha256, Sha512};
use std::fmt;

lazy_static! {
	/// List of bip39 words
	pub static ref WORDS: Vec<String> = include_str!("wordlists/en.txt").split_whitespace().map(|s| s.into()).collect();
}

/// An error that might occur during mnemonic decoding
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Error {
	/// Invalid word encountered
	BadWord(String),
	/// Checksum was not correct (expected, actual)
	BadChecksum(u8, u8),
	/// The number of words/bytes was invalid
	InvalidLength(usize),
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match *self {
			Error::BadWord(ref b) => write!(f, "invalid bip39 word {}", b),
			Error::BadChecksum(exp, actual) => write!(
				f,
				"checksum 0x{:x} does not match expected 0x{:x}",
				actual, exp
			),
			Error::InvalidLength(ell) => write!(f, "invalid mnemonic/entropy length {}", ell),
		}
	}
}

/// Returns the index of a word in the wordlist
pub fn search(word: &str) -> Result<u16, Error> {
	let w = word.to_string();
	match WORDS.binary_search(&w) {
		Ok(index) => Ok(index as u16),
		Err(_) => Err(Error::BadWord(w)),
	}
}

/// Converts a mnemonic to entropy
pub fn to_entropy(mnemonic: &str) -> Result<Vec<u8>, Error> {
	let words: Vec<String> = mnemonic.split_whitespace().map(|s| s.into()).collect();

	let sizes: [usize; 5] = [12, 15, 18, 21, 24];
	if !sizes.contains(&words.len()) {
		return Err(Error::InvalidLength(words.len()));
	}

	// u11 vector of indexes for each word
	let mut indexes: Vec<u16> = words
		.iter()
		.map(|x| search(x))
		.collect::<Result<Vec<_>, _>>()?;
	let checksum_bits = words.len() / 3;
	let mask = ((1 << checksum_bits) - 1) as u8;
	let last = indexes.pop().unwrap();
	let checksum = (last as u8) & mask;

	let datalen = ((11 * words.len()) - checksum_bits) / 8 - 1;
	let mut entropy: Vec<u8> = vec![0; datalen];
	// set the last byte to the data part of the last word
	entropy.push((last >> checksum_bits) as u8);
	// start setting bits from this index
	let mut loc: usize = 11 - checksum_bits;

	// cast vector of u11 as u8
	for index in indexes.iter().rev() {
		for i in 0..11 {
			let bit = index & (1 << i) != 0;
			entropy[datalen - loc / 8] |= (bit as u8) << (loc % 8);
			loc += 1;
		}
	}

	let mut hash = [0; 32];
	let mut sha2sum = Sha256::default();
	sha2sum.update(&entropy);
	hash.copy_from_slice(sha2sum.finalize().as_slice());

	let actual = (hash[0] >> (8 - checksum_bits)) & mask;

	if actual != checksum {
		return Err(Error::BadChecksum(checksum, actual));
	}

	Ok(entropy)
}

/// Converts entropy to a mnemonic
pub fn from_entropy(entropy: &[u8]) -> Result<String, Error> {
	let sizes: [usize; 5] = [16, 20, 24, 28, 32];
	let length = entropy.len();
	if !sizes.contains(&length) {
		return Err(Error::InvalidLength(length));
	}

	let checksum_bits = length / 4;
	let mask = ((1 << checksum_bits) - 1) as u8;

	let mut hash = [0; 32];
	let mut sha2sum = Sha256::default();
	sha2sum.update(entropy);
	hash.copy_from_slice(sha2sum.finalize().as_slice());

	let checksum = (hash[0] >> 8 - checksum_bits) & mask;

	let nwords = (length * 8 + checksum_bits) / 11;
	let mut indexes: Vec<u16> = vec![0; nwords];
	let mut loc: usize = 0;

	// u8 to u11
	for byte in entropy.iter() {
		for i in (0..8).rev() {
			let bit = byte & (1 << i) != 0;
			indexes[loc / 11] |= (bit as u16) << (10 - (loc % 11));
			loc += 1;
		}
	}
	for i in (0..checksum_bits).rev() {
		let bit = checksum & (1 << i) != 0;
		indexes[loc / 11] |= (bit as u16) << (10 - (loc % 11));
		loc += 1;
	}

	let words: Vec<String> = indexes.iter().map(|x| WORDS[*x as usize].clone()).collect();
	let mnemonic = words.join(" ");
	Ok(mnemonic)
}

/// Converts a nemonic and a passphrase into a seed
pub fn to_seed<'a, T: 'a>(mnemonic: &str, passphrase: T) -> Result<[u8; 64], Error>
where
	Option<&'a str>: From<T>,
{
	// make sure the mnemonic is valid
	to_entropy(mnemonic)?;

	let salt = ("mnemonic".to_owned() + Option::from(passphrase).unwrap_or("")).into_bytes();
	let data = mnemonic.as_bytes();
	let mut seed = [0; 64];

	pbkdf2::<Hmac<Sha512>>(data, &salt[..], 2048, &mut seed);

	Ok(seed)
}

#[cfg(test)]
mod tests {
	use super::{from_entropy, to_entropy, to_seed};
	use crate::util::{from_hex, ToHex};
	use rand::{thread_rng, Rng};

	struct Test<'a> {
		mnemonic: &'a str,
		entropy: &'a str,
		seed: &'a str,
	}

	/// Test vectors from https://github.com/bitcoin/bips/blob/master/bip-0039.mediawiki#Test_vectors
	fn tests<'a>() -> Vec<Test<'a>> {
		vec![
            Test {
                entropy: "00000000000000000000000000000000",
                mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
                seed: "c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04",
            },
            Test {
                entropy: "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
                mnemonic: "legal winner thank year wave sausage worth useful legal winner thank yellow",
                seed: "2e8905819b8723fe2c1d161860e5ee1830318dbf49a83bd451cfb8440c28bd6fa457fe1296106559a3c80937a1c1069be3a3a5bd381ee6260e8d9739fce1f607",
            },
            Test {
                entropy: "80808080808080808080808080808080",
                mnemonic: "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
                seed: "d71de856f81a8acc65e6fc851a38d4d7ec216fd0796d0a6827a3ad6ed5511a30fa280f12eb2e47ed2ac03b5c462a0358d18d69fe4f985ec81778c1b370b652a8",
            },
            Test {
                entropy: "ffffffffffffffffffffffffffffffff",
                mnemonic: "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
                seed: "ac27495480225222079d7be181583751e86f571027b0497b5b5d11218e0a8a13332572917f0f8e5a589620c6f15b11c61dee327651a14c34e18231052e48c069",
            },
            Test {
                entropy: "000000000000000000000000000000000000000000000000",
                mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon agent",
                seed: "035895f2f481b1b0f01fcf8c289c794660b289981a78f8106447707fdd9666ca06da5a9a565181599b79f53b844d8a71dd9f439c52a3d7b3e8a79c906ac845fa",
            },
            Test {
                entropy: "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
                mnemonic: "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal will",
                seed: "f2b94508732bcbacbcc020faefecfc89feafa6649a5491b8c952cede496c214a0c7b3c392d168748f2d4a612bada0753b52a1c7ac53c1e93abd5c6320b9e95dd",
            },
            Test {
                entropy: "808080808080808080808080808080808080808080808080",
                mnemonic: "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter always",
                seed: "107d7c02a5aa6f38c58083ff74f04c607c2d2c0ecc55501dadd72d025b751bc27fe913ffb796f841c49b1d33b610cf0e91d3aa239027f5e99fe4ce9e5088cd65",
            },
            Test {
                entropy: "ffffffffffffffffffffffffffffffffffffffffffffffff",
                mnemonic: "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo when",
                seed: "0cd6e5d827bb62eb8fc1e262254223817fd068a74b5b449cc2f667c3f1f985a76379b43348d952e2265b4cd129090758b3e3c2c49103b5051aac2eaeb890a528",
            },
            Test {
                entropy: "0000000000000000000000000000000000000000000000000000000000000000",
                mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
                seed: "bda85446c68413707090a52022edd26a1c9462295029f2e60cd7c4f2bbd3097170af7a4d73245cafa9c3cca8d561a7c3de6f5d4a10be8ed2a5e608d68f92fcc8",
            },
            Test {
                entropy: "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
                mnemonic: "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title",
                seed: "bc09fca1804f7e69da93c2f2028eb238c227f2e9dda30cd63699232578480a4021b146ad717fbb7e451ce9eb835f43620bf5c514db0f8add49f5d121449d3e87",
            },
            Test {
                entropy: "8080808080808080808080808080808080808080808080808080808080808080",
                mnemonic: "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic bless",
                seed: "c0c519bd0e91a2ed54357d9d1ebef6f5af218a153624cf4f2da911a0ed8f7a09e2ef61af0aca007096df430022f7a2b6fb91661a9589097069720d015e4e982f",
            },
            Test {
                entropy: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                mnemonic: "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote",
                seed: "dd48c104698c30cfe2b6142103248622fb7bb0ff692eebb00089b32d22484e1613912f0a5b694407be899ffd31ed3992c456cdf60f5d4564b8ba3f05a69890ad",
            },
            Test {
                entropy: "9e885d952ad362caeb4efe34a8e91bd2",
                mnemonic: "ozone drill grab fiber curtain grace pudding thank cruise elder eight picnic",
                seed: "274ddc525802f7c828d8ef7ddbcdc5304e87ac3535913611fbbfa986d0c9e5476c91689f9c8a54fd55bd38606aa6a8595ad213d4c9c9f9aca3fb217069a41028",
            },
            Test {
                entropy: "6610b25967cdcca9d59875f5cb50b0ea75433311869e930b",
                mnemonic: "gravity machine north sort system female filter attitude volume fold club stay feature office ecology stable narrow fog",
                seed: "628c3827a8823298ee685db84f55caa34b5cc195a778e52d45f59bcf75aba68e4d7590e101dc414bc1bbd5737666fbbef35d1f1903953b66624f910feef245ac",
            },
            Test {
                entropy: "68a79eaca2324873eacc50cb9c6eca8cc68ea5d936f98787c60c7ebc74e6ce7c",
                mnemonic: "hamster diagram private dutch cause delay private meat slide toddler razor book happy fancy gospel tennis maple dilemma loan word shrug inflict delay length",
                seed: "64c87cde7e12ecf6704ab95bb1408bef047c22db4cc7491c4271d170a1b213d20b385bc1588d9c7b38f1b39d415665b8a9030c9ec653d75e65f847d8fc1fc440",
            },
            Test {
                entropy: "c0ba5a8e914111210f2bd131f3d5e08d",
                mnemonic: "scheme spot photo card baby mountain device kick cradle pact join borrow",
                seed: "ea725895aaae8d4c1cf682c1bfd2d358d52ed9f0f0591131b559e2724bb234fca05aa9c02c57407e04ee9dc3b454aa63fbff483a8b11de949624b9f1831a9612",
            },
            Test {
                entropy: "6d9be1ee6ebd27a258115aad99b7317b9c8d28b6d76431c3",
                mnemonic: "horn tenant knee talent sponsor spell gate clip pulse soap slush warm silver nephew swap uncle crack brave",
                seed: "fd579828af3da1d32544ce4db5c73d53fc8acc4ddb1e3b251a31179cdb71e853c56d2fcb11aed39898ce6c34b10b5382772db8796e52837b54468aeb312cfc3d",
            },
            Test {
                entropy: "9f6a2878b2520799a44ef18bc7df394e7061a224d2c33cd015b157d746869863",
                mnemonic: "panda eyebrow bullet gorilla call smoke muffin taste mesh discover soft ostrich alcohol speed nation flash devote level hobby quick inner drive ghost inside",
                seed: "72be8e052fc4919d2adf28d5306b5474b0069df35b02303de8c1729c9538dbb6fc2d731d5f832193cd9fb6aeecbc469594a70e3dd50811b5067f3b88b28c3e8d",
            },
            Test {
                entropy: "23db8160a31d3e0dca3688ed941adbf3",
                mnemonic: "cat swing flag economy stadium alone churn speed unique patch report train",
                seed: "deb5f45449e615feff5640f2e49f933ff51895de3b4381832b3139941c57b59205a42480c52175b6efcffaa58a2503887c1e8b363a707256bdd2b587b46541f5",
            },
            Test {
                entropy: "8197a4a47f0425faeaa69deebc05ca29c0a5b5cc76ceacc0",
                mnemonic: "light rule cinnamon wrap drastic word pride squirrel upgrade then income fatal apart sustain crack supply proud access",
                seed: "4cbdff1ca2db800fd61cae72a57475fdc6bab03e441fd63f96dabd1f183ef5b782925f00105f318309a7e9c3ea6967c7801e46c8a58082674c860a37b93eda02",
            },
            Test {
                entropy: "066dca1a2bb7e8a1db2832148ce9933eea0f3ac9548d793112d9a95c9407efad",
                mnemonic: "all hour make first leader extend hole alien behind guard gospel lava path output census museum junior mass reopen famous sing advance salt reform",
                seed: "26e975ec644423f4a4c4f4215ef09b4bd7ef924e85d1d17c4cf3f136c2863cf6df0a475045652c57eb5fb41513ca2a2d67722b77e954b4b3fc11f7590449191d",
            },
            Test {
                entropy: "f30f8c1da665478f49b001d94c5fc452",
                mnemonic: "vessel ladder alter error federal sibling chat ability sun glass valve picture",
                seed: "2aaa9242daafcee6aa9d7269f17d4efe271e1b9a529178d7dc139cd18747090bf9d60295d0ce74309a78852a9caadf0af48aae1c6253839624076224374bc63f",
            },
            Test {
                entropy: "c10ec20dc3cd9f652c7fac2f1230f7a3c828389a14392f05",
                mnemonic: "scissors invite lock maple supreme raw rapid void congress muscle digital elegant little brisk hair mango congress clump",
                seed: "7b4a10be9d98e6cba265566db7f136718e1398c71cb581e1b2f464cac1ceedf4f3e274dc270003c670ad8d02c4558b2f8e39edea2775c9e232c7cb798b069e88",
            },
            Test {
                entropy: "f585c11aec520db57dd353c69554b21a89b20fb0650966fa0a9d6f74fd989d8f",
                mnemonic: "void come effort suffer camp survey warrior heavy shoot primary clutch crush open amazing screen patrol group space point ten exist slush involve unfold",
                seed: "01f5bced59dec48e362f2c45b5de68b9fd6c92c6634f44d6d40aab69056506f0e35524a518034ddc1192e1dacd32c1ed3eaa3c3b131c88ed8e7e54c49a5d0998",
            }
        ]
	}
	#[test]
	fn test_bip39() {
		let tests = tests();
		for t in tests.iter() {
			assert_eq!(
				(&to_seed(t.mnemonic, "TREZOR").unwrap()[..]).to_hex(),
				t.seed.to_string()
			);
			assert_eq!(
				to_entropy(t.mnemonic).unwrap().to_vec(),
				from_hex(t.entropy).unwrap()
			);
			assert_eq!(
				from_entropy(&from_hex(t.entropy).unwrap()).unwrap(),
				t.mnemonic
			);
		}
	}

	#[test]
	fn test_bip39_random() {
		use rand::seq::SliceRandom;
		let sizes: [usize; 5] = [16, 20, 24, 28, 32];

		let mut rng = thread_rng();
		let size = *sizes.choose(&mut rng).unwrap();
		let mut entropy: Vec<u8> = Vec::with_capacity(size);

		for _ in 0..size {
			let val: u8 = rng.gen();
			entropy.push(val);
		}

		assert_eq!(
			entropy,
			to_entropy(&from_entropy(&entropy).unwrap()).unwrap()
		)
	}

	#[test]
	fn test_invalid() {
		// Invalid words
		assert!(to_entropy("this is not a love song this is not a love song").is_err());
		assert!(to_entropy("abandon abandon badword abandon abandon abandon abandon abandon abandon abandon abandon abandon").is_err());
		// Invalid length
		assert!(to_entropy("abandon abandon abandon abandon abandon abandon").is_err());
		assert!(from_entropy(&vec![1, 2, 3, 4, 5]).is_err());
		assert!(from_entropy(&vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]).is_err());
		// Invalid checksum
		assert!(to_entropy("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon").is_err());
		assert!(to_entropy("zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo").is_err());
		assert!(to_entropy("scissors invite lock maple supreme raw rapid void congress muscle digital elegant little brisk hair mango congress abandon").is_err());
	}
}
