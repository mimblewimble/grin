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
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Delete a directory or file
pub fn delete(path_buf: PathBuf) -> io::Result<()> {
	if path_buf.is_dir() {
		fs::remove_dir_all(path_buf)
	} else if path_buf.is_file() {
		fs::remove_file(path_buf)
	} else {
		Ok(())
	}
}

/// Copy directory, create destination if needed
pub fn copy_dir_to(src: &Path, dst: &Path) -> io::Result<u64> {
	let mut counter = 0u64;
	if !dst.is_dir() {
		fs::create_dir(dst)?
	}

	for entry_result in src.read_dir()? {
		let entry = entry_result?;
		let file_type = entry.file_type()?;
		let count = copy_to(&entry.path(), file_type, &dst.join(entry.file_name()))?;
		counter += count;
	}
	Ok(counter)
}

/// List directory
pub fn list_files(path: &Path) -> Vec<PathBuf> {
	WalkDir::new(path)
		.sort_by(|a, b| a.path().cmp(b.path()))
		.min_depth(1)
		.into_iter()
		.filter_map(|x| x.ok())
		.filter(|x| x.file_type().is_file())
		.filter_map(|x| x.path().strip_prefix(path).map(|x| x.to_path_buf()).ok())
		.collect()
}

fn copy_to(src: &Path, src_type: fs::FileType, dst: &Path) -> io::Result<u64> {
	if src_type.is_file() {
		fs::copy(src, dst)
	} else if src_type.is_dir() {
		copy_dir_to(src, dst)
	} else {
		Err(io::Error::new(
			io::ErrorKind::Other,
			format!("Could not copy: {}", src.display()),
		))
	}
}

/// Retrieve first line from file
pub fn get_first_line(file_path: Option<String>) -> Option<String> {
	file_path.and_then(|path| match fs::File::open(path) {
		Ok(file) => {
			let buf_reader = io::BufReader::new(file);
			let mut lines_iter = buf_reader.lines().map(|l| l.unwrap());
			lines_iter.next()
		}
		Err(_) => None,
	})
}
