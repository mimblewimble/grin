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

use grin_util as util;

use crate::util::zip;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

#[test]
fn zip_unzip() {
	let root = Path::new("./target/tmp");
	let zip_name = "./target/tmp/zipped.zip";

	fs::create_dir_all(root.join("./to_zip/sub")).unwrap();
	write_files("to_zip".to_string(), &root).unwrap();

	let zip_file = File::create(zip_name).unwrap();
	zip::compress(&root.join("./to_zip"), &zip_file).unwrap();
	zip_file.sync_all().unwrap();

	let zip_path = Path::new(zip_name);
	assert!(zip_path.exists());
	assert!(zip_path.is_file());
	assert!(zip_path.metadata().unwrap().len() > 300);

	fs::create_dir_all(root.join("./dezipped")).unwrap();
	let zip_file = File::open(zip_name).unwrap();
	zip::decompress(zip_file, &root.join("./dezipped"), |_| true).unwrap();

	assert!(root.join("to_zip/foo.txt").is_file());
	assert!(root.join("to_zip/bar.txt").is_file());
	assert!(root.join("to_zip/sub").is_dir());
	let lorem = root.join("to_zip/sub/lorem");
	assert!(lorem.is_file());
	assert!(lorem.metadata().unwrap().len() == 55);

	let decompressed = zip::decompress(
		File::open("tests/test.zip").unwrap(),
		&root.join("./dezipped"),
		|_| true,
	)
	.unwrap();
	assert_eq!(decompressed, 1);
}

fn write_files(dir_name: String, root: &Path) -> io::Result<()> {
	let mut file = File::create(root.join(dir_name.clone() + "/foo.txt"))?;
	file.write_all(b"Hello, world!")?;
	let mut file = File::create(root.join(dir_name.clone() + "/bar.txt"))?;
	file.write_all(b"Goodbye, world!")?;
	let mut file = File::create(root.join(dir_name.clone() + "/sub/lorem"))?;
	file.write_all(b"Lorem ipsum dolor sit amet, consectetur adipiscing elit")?;
	Ok(())
}
