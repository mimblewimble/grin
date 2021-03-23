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

/// Wrappers around the `zip-rs` library to compress and decompress zip archives.
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::thread;

use self::zip_rs::write::FileOptions;
use zip as zip_rs;

// Sanitize file path for normal components, excluding '/', '..', and '.'
// From private function in zip crate
fn path_to_string(path: &std::path::Path) -> String {
    let mut path_str = String::new();
    for component in path.components() {
        if let std::path::Component::Normal(os_str) = component {
            if !path_str.is_empty() {
                path_str.push('/');
            }
            path_str.push_str(&*os_str.to_string_lossy());
        }
    }
    path_str
}

/// Create a zip archive from source dir and list of relative file paths.
/// Permissions are set to 644 by default.
pub fn create_zip(dst_file: &File, src_dir: &Path, files: Vec<PathBuf>) -> io::Result<()> {
	let mut writer = {
		let zip = zip_rs::ZipWriter::new(dst_file);
		BufWriter::new(zip)
	};

	let options = FileOptions::default()
		.compression_method(zip_rs::CompressionMethod::Stored)
		.unix_permissions(0o644);

	for x in &files {
		let file_path = src_dir.join(x);
		if let Ok(file) = File::open(file_path.clone()) {
			info!("compress: {:?} -> {:?}", file_path, x);
			writer.get_mut().start_file(path_to_string(x), options)?;
			io::copy(&mut BufReader::new(file), &mut writer)?;
			// Flush the BufWriter after each file so we start then next one correctly.
			writer.flush()?;
		}
	}

	writer.get_mut().finish()?;
	dst_file.sync_all()?;
	Ok(())
}

/// Extract a set of files from the provided zip archive.
pub fn extract_files(from_archive: File, dest: &Path, files: Vec<PathBuf>) -> io::Result<()> {
	let dest: PathBuf = PathBuf::from(dest);
	let files: Vec<_> = files.to_vec();
	let res = thread::spawn(move || {
		let mut archive = zip_rs::ZipArchive::new(from_archive).expect("archive file exists");
		for x in files {
			if let Ok(file) = archive.by_name(x.to_str().expect("valid path")) {
				let path = dest.join(file.mangled_name());
				let parent_dir = path.parent().expect("valid parent dir");
				fs::create_dir_all(&parent_dir).expect("create parent dir");
				let outfile = fs::File::create(&path).expect("file created");
				io::copy(&mut BufReader::new(file), &mut BufWriter::new(outfile))
					.expect("write to file");

				info!("extract_files: {:?} -> {:?}", x, path);

				// Set file permissions to "644" (Unix only).
				#[cfg(unix)]
				{
					use std::os::unix::fs::PermissionsExt;
					let mode = PermissionsExt::from_mode(0o644);
					fs::set_permissions(&path, mode).expect("set file permissions");
				}
			}
		}
	})
	.join();

	// If join() above is Ok then we successfully extracted the files.
	// If the result is Err then we failed to extract the files.
	res.map_err(|e| {
		error!("failed to extract files from zip: {:?}", e);
		io::Error::new(io::ErrorKind::Other, "failed to extract files from zip")
	})
}
