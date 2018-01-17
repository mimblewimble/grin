// Copyright 2016 The Grin Developers
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

use openssl::asn1::Asn1Time;
use openssl::error::ErrorStack;
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey};
use openssl::rsa::Rsa;
use openssl::x509::{X509, X509Name};
use openssl::x509::extension::{KeyUsage};
use openssl::pkcs12::{Pkcs12};
use openssl::nid;

use std::fs::File;
use std::path::Path;
use std::io::Write;

pub fn generate_certs(filename: String) -> Result<(), ErrorStack> {

	let subject_name = "localhost";
	let rsa = Rsa::generate(2048).unwrap();
	let pkey = PKey::from_rsa(rsa).unwrap();

	let mut name = X509Name::builder().unwrap();
	name.append_entry_by_nid(nid::COMMONNAME, subject_name)
		.unwrap();
	let name = name.build();

	let key_usage = KeyUsage::new().digital_signature().build().unwrap();

	let mut builder = X509::builder().unwrap();
	builder.set_version(2).unwrap();
	builder
		.set_not_before(&Asn1Time::days_from_now(0).unwrap())
		.unwrap();
	builder
		.set_not_after(&Asn1Time::days_from_now(365).unwrap())
		.unwrap();
	builder.set_subject_name(&name).unwrap();
	builder.set_issuer_name(&name).unwrap();
	builder.append_extension(key_usage).unwrap();
	builder.set_pubkey(&pkey).unwrap();
	builder.sign(&pkey, MessageDigest::sha256()).unwrap();
	let cert = builder.build();


	let file_path = Path::new(&filename);
	let mut file = File::create(file_path).unwrap();

	let pkcs12_builder = Pkcs12::builder();
	let pkcs12 = pkcs12_builder
		.build("", "localhost", &pkey, &cert)
		.unwrap();

	let der = pkcs12.to_der().unwrap();

	file.write_all(&der).unwrap();

	Ok(())
}