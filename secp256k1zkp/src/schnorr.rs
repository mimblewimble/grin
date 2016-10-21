// Bitcoin secp256k1 bindings
// Written in 2014 by
//   Dawid Ciężarkiewicz
//   Andrew Poelstra
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

//! # Schnorr signatures

use ContextFlag;
use Error;
use Message;
use Secp256k1;

use constants;
use ffi;
use key::{SecretKey, PublicKey};

use std::{mem, ptr};

/// A Schnorr signature.
pub struct Signature([u8; constants::SCHNORR_SIGNATURE_SIZE]);
impl_array_newtype!(Signature, u8, constants::SCHNORR_SIGNATURE_SIZE);
impl_pretty_debug!(Signature);

impl Signature {
    /// Deserializes a signature from a 64-byte vector
    pub fn deserialize(data: &[u8]) -> Signature {
        assert_eq!(data.len(), constants::SCHNORR_SIGNATURE_SIZE);
        unsafe {
            let mut ret: Signature = mem::uninitialized();
            ptr::copy_nonoverlapping(data.as_ptr(),
                                     ret.as_mut_ptr(),
                                     constants::SCHNORR_SIGNATURE_SIZE);
            ret
        }
    }

    /// Serializes a signature to a 64-byte vector
    pub fn serialize(&self) -> Vec<u8> {
        let mut ret = Vec::with_capacity(constants::SCHNORR_SIGNATURE_SIZE);
        unsafe {
            ptr::copy_nonoverlapping(self.as_ptr(),
                                     ret.as_mut_ptr(),
                                     constants::SCHNORR_SIGNATURE_SIZE);
            ret.set_len(constants::SCHNORR_SIGNATURE_SIZE);
        }
        ret
    }
}

impl Secp256k1 {
    /// Create a Schnorr signature
    pub fn sign_schnorr(&self, msg: &Message, sk: &SecretKey) -> Result<Signature, Error> {
        if self.caps == ContextFlag::VerifyOnly || self.caps == ContextFlag::None {
            return Err(Error::IncapableContext);
        }

        let mut ret: Signature = unsafe { mem::uninitialized() };
        unsafe {
            // We can assume the return value because it's not possible to construct
            // an invalid signature from a valid `Message` and `SecretKey`
            let err = ffi::secp256k1_schnorr_sign(self.ctx,
                                                  ret.as_mut_ptr(),
                                                  msg.as_ptr(),
                                                  sk.as_ptr(),
                                                  ffi::secp256k1_nonce_function_rfc6979,
                                                  ptr::null());
            debug_assert_eq!(err, 1);
        }
        Ok(ret)
    }

    /// Verify a Schnorr signature
    pub fn verify_schnorr(&self,
                          msg: &Message,
                          sig: &Signature,
                          pk: &PublicKey)
                          -> Result<(), Error> {
        if self.caps == ContextFlag::SignOnly || self.caps == ContextFlag::None {
            return Err(Error::IncapableContext);
        }

        if !pk.is_valid() {
            Err(Error::InvalidPublicKey)
        } else if unsafe {
            ffi::secp256k1_schnorr_verify(self.ctx, sig.as_ptr(), msg.as_ptr(), pk.as_ptr())
        } == 0 {
            Err(Error::IncorrectSignature)
        } else {
            Ok(())
        }
    }

    /// Retrieves the public key for which `sig` is a valid signature for `msg`.
    /// Requires a verify-capable context.
    pub fn recover_schnorr(&self, msg: &Message, sig: &Signature) -> Result<PublicKey, Error> {
        if self.caps == ContextFlag::SignOnly || self.caps == ContextFlag::None {
            return Err(Error::IncapableContext);
        }

        let mut pk = unsafe { ffi::PublicKey::blank() };
        unsafe {
            if ffi::secp256k1_schnorr_recover(self.ctx, &mut pk, sig.as_ptr(), msg.as_ptr()) != 1 {
                return Err(Error::InvalidSignature);
            }
        };
        Ok(PublicKey::from(pk))
    }
}

#[cfg(test)]
mod tests {
    use rand::{Rng, thread_rng};
    use ContextFlag;
    use Message;
    use Secp256k1;
    use Error::IncapableContext;
    use super::Signature;

    #[test]
    fn capabilities() {
        let none = Secp256k1::with_caps(ContextFlag::None);
        let sign = Secp256k1::with_caps(ContextFlag::SignOnly);
        let vrfy = Secp256k1::with_caps(ContextFlag::VerifyOnly);
        let full = Secp256k1::with_caps(ContextFlag::Full);

        let mut msg = [0u8; 32];
        thread_rng().fill_bytes(&mut msg);
        let msg = Message::from_slice(&msg).unwrap();

        let (sk, pk) = full.generate_keypair(&mut thread_rng()).unwrap();

        // Try signing
        assert_eq!(none.sign_schnorr(&msg, &sk), Err(IncapableContext));
        assert_eq!(vrfy.sign_schnorr(&msg, &sk), Err(IncapableContext));
        assert!(sign.sign_schnorr(&msg, &sk).is_ok());
        assert!(full.sign_schnorr(&msg, &sk).is_ok());
        assert_eq!(sign.sign_schnorr(&msg, &sk), full.sign_schnorr(&msg, &sk));
        let sig = full.sign_schnorr(&msg, &sk).unwrap();

        // Try verifying
        assert_eq!(none.verify_schnorr(&msg, &sig, &pk), Err(IncapableContext));
        assert_eq!(sign.verify_schnorr(&msg, &sig, &pk), Err(IncapableContext));
        assert!(vrfy.verify_schnorr(&msg, &sig, &pk).is_ok());
        assert!(full.verify_schnorr(&msg, &sig, &pk).is_ok());

        // Try pk recovery
        assert_eq!(none.recover_schnorr(&msg, &sig), Err(IncapableContext));
        assert_eq!(sign.recover_schnorr(&msg, &sig), Err(IncapableContext));
        assert!(vrfy.recover_schnorr(&msg, &sig).is_ok());
        assert!(full.recover_schnorr(&msg, &sig).is_ok());

        assert_eq!(vrfy.recover_schnorr(&msg, &sig),
                   full.recover_schnorr(&msg, &sig));
        assert_eq!(full.recover_schnorr(&msg, &sig), Ok(pk));
    }

    #[test]
    fn sign_verify() {
        let mut s = Secp256k1::new();
        s.randomize(&mut thread_rng());

        let mut msg = [0u8; 32];
        thread_rng().fill_bytes(&mut msg);
        let msg = Message::from_slice(&msg).unwrap();

        let (sk, pk) = s.generate_keypair(&mut thread_rng()).unwrap();

        let sig = s.sign_schnorr(&msg, &sk).unwrap();
        assert!(s.verify_schnorr(&msg, &sig, &pk).is_ok());
    }

    #[test]
    fn deserialize() {
        let mut s = Secp256k1::new();
        s.randomize(&mut thread_rng());

        let mut msg = [0u8; 32];
        thread_rng().fill_bytes(&mut msg);
        let msg = Message::from_slice(&msg).unwrap();

        let (sk, _) = s.generate_keypair(&mut thread_rng()).unwrap();

        let sig1 = s.sign_schnorr(&msg, &sk).unwrap();
        let sig2 = Signature::deserialize(&sig1.serialize());
        assert_eq!(sig1, sig2);
    }
}
