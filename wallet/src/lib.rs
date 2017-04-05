
extern crate secp256k1zkp as secp;
extern crate crypto;
extern crate rand;
extern crate byteorder;

use std::{error, fmt};

pub mod extendedkey;
pub mod constants;

/// An ExtKey error
#[derive(Copy, PartialEq, Eq, Clone, Debug)]
pub enum Error {
    /// The size of the seed is invalid
    InvalidSeedSize,
    InvalidSliceSize,
    InvalidExtendedKey,
}

// Passthrough Debug to Display, since errors should be user-visible
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(error::Error::description(self))
    }
}

impl error::Error for Error {
    fn cause(&self) -> Option<&error::Error> {
        None
    }

    fn description(&self) -> &str {
        match *self {
            Error::InvalidSeedSize => "wallet: seed isn't of size 128, 256 or 512",
            //TODO change when ser. ext. size is fixed
            Error::InvalidSliceSize => "wallet: serialized extended key must be of size 73",
            Error::InvalidExtendedKey => "wallet: the given serialized extended key is invalid",
        }
    }
}
