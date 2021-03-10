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

use grin_util as util;

use crate::util::zip;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[test]
fn zip_unzip() {
	let root = Path::new("target/tmp");
	let zip_path = root.join("zipped.zip");
	let path = root.join("to_zip");

	// Some files we want to use for testing our zip file.
	{
		fs::create_dir_all(&path).unwrap();

		let mut file = File::create(path.join("foo.txt")).unwrap();
		file.write_all(b"Hello, world!").unwrap();

		let mut file = File::create(path.join("bar.txt")).unwrap();
		file.write_all(b"This, was unexpected!").unwrap();

		let mut file = File::create(path.join("wat.txt")).unwrap();
		file.write_all(b"Goodbye, world!").unwrap();

		let sub_path = path.join("sub");
		fs::create_dir_all(&sub_path).unwrap();
		let mut file = File::create(sub_path.join("lorem.txt")).unwrap();
		file.write_all(b"Lorem ipsum dolor sit amet, consectetur adipiscing elit")
			.unwrap();
	}

	// Create our zip file using an explicit (sub)set of the above files.
	{
		// List of files to be accepted when creating the zip and extracting from the zip.
		// Note: "wat.txt" is not included in the list of files (hence it is excluded).
		let files = vec![
			PathBuf::from("foo.txt"),
			PathBuf::from("bar.txt"),
			PathBuf::from("sub/lorem.txt"),
		];

		let zip_file = File::create(&zip_path).unwrap();
		zip::create_zip(&zip_file, &path, files).unwrap();
		zip_file.sync_all().unwrap();
	}

	assert!(zip_path.exists());
	assert!(zip_path.is_file());
	assert!(zip_path.metadata().unwrap().len() > 300);

	let zip_file = File::open(zip_path).unwrap();

	{
		let dest_dir = root.join("unzipped");
		fs::create_dir_all(&dest_dir).unwrap();

		// List of files to extract from the zip.
		// Note: we do not extract "wat.txt" here, even if present in the zip.
		let files = vec![PathBuf::from("foo.txt"), PathBuf::from("sub/lorem.txt")];

		zip::extract_files(zip_file, &dest_dir, files).unwrap();

		assert!(dest_dir.join("foo.txt").is_file());

		// Check we did not extract "bar.txt" from the zip file.
		// We should *only* extract the files explicitly listed.
		assert!(!dest_dir.join("bar.txt").exists());

		let sub_path = dest_dir.join("sub");
		assert!(sub_path.is_dir());

		let lorem = sub_path.join("lorem.txt");
		assert!(lorem.is_file());
		assert_eq!(
			fs::read_to_string(lorem).unwrap(),
			"Lorem ipsum dolor sit amet, consectetur adipiscing elit"
		);
	}
}
