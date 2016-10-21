// Bitcoin secp256k1 bindings
// Written in 2015 by
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

//! # Build script

// Coding conventions
#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

extern crate gcc;

fn main() {
    let mut base_config = gcc::Config::new();
    base_config.include("depend/secp256k1-zkp/")
               .include("depend/secp256k1-zkp/include")
               .include("depend/secp256k1-zkp/src")
               .flag("-g")
               // TODO these three should be changed to use libgmp, at least until secp PR 290 is merged
               .define("USE_NUM_NONE", Some("1"))
               .define("USE_FIELD_INV_BUILTIN", Some("1"))
               .define("USE_SCALAR_INV_BUILTIN", Some("1"))
               // TODO these should use 64-bit variants on 64-bit systems
               .define("USE_FIELD_10X26", Some("1"))
               .define("USE_SCALAR_8X32", Some("1"))
               .define("USE_ENDOMORPHISM", Some("1"))
               // These all are OK.
               .define("ENABLE_MODULE_ECDH", Some("1"))
               .define("ENABLE_MODULE_SCHNORR", Some("1"))
               .define("ENABLE_MODULE_RECOVERY", Some("1"))
               .define("ENABLE_MODULE_RANGEPROOF", Some("1"));

    // secp256k1
    base_config.file("depend/secp256k1-zkp/contrib/lax_der_parsing.c")
        .file("depend/secp256k1-zkp/src/secp256k1.c")
        .compile("libsecp256k1-zkp.a");
}
