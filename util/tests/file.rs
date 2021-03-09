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

use self::util::file;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

#[test]
fn copy_dir() {
	let root = Path::new("./target/tmp2");
	fs::create_dir_all(root.join("./original/sub")).unwrap();
	fs::create_dir_all(root.join("./original/sub2")).unwrap();
	write_files("original".to_string(), &root).unwrap();
	let original_path = Path::new("./target/tmp2/original");
	let copy_path = Path::new("./target/tmp2/copy");
	file::copy_dir_to(original_path, copy_path).unwrap();
	let original_files = file::list_files(&Path::new("./target/tmp2/original"));
	let copied_files = file::list_files(&Path::new("./target/tmp2/copy"));
	assert_eq!(original_files, copied_files);
	fs::remove_dir_all(root).unwrap();
}

fn write_files(dir_name: String, root: &Path) -> io::Result<()> {
	let mut file = File::create(root.join(dir_name.clone() + "/foo.txt"))?;
	file.write_all(b"Hello, world!")?;
	let mut file = File::create(root.join(dir_name.clone() + "/bar.txt"))?;
	file.write_all(b"Goodbye, world!")?;
	let mut file = File::create(root.join(dir_name + "/sub/lorem"))?;
	file.write_all(b"Lorem ipsum dolor sit amet, consectetur adipiscing elit")?;
	Ok(())
}
