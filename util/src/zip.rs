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

/// Wrappers around the `zip-rs` library to compress and decompress zip archives.
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::thread;
use walkdir::WalkDir;

use self::zip_rs::result::{ZipError, ZipResult};
use self::zip_rs::write::FileOptions;
use zip as zip_rs;

/// Compress a source directory recursively into a zip file.
/// Permissions are set to 644 by default to avoid any
/// unwanted execution bits.
///
/// TODO - Pass in a list of files to include in the zip (similar to extract_files).
/// We do not need (or want) to walk the dir and include everything up here.
///
pub fn compress(src_dir: &Path, dst_file: &File) -> ZipResult<()> {
	if !Path::new(src_dir).is_dir() {
		return Err(ZipError::Io(io::Error::new(
			io::ErrorKind::Other,
			"Source must be a directory.",
		)));
	}

	let options = FileOptions::default()
		.compression_method(zip_rs::CompressionMethod::Stored)
		.unix_permissions(0o644);

	let mut zip = zip_rs::ZipWriter::new(dst_file);
	let walkdir = WalkDir::new(src_dir.to_str().unwrap());
	let it = walkdir.into_iter();

	for dent in it.filter_map(|e| e.ok()) {
		let path = dent.path();
		let name = path
			.strip_prefix(Path::new(src_dir))
			.unwrap()
			.to_str()
			.unwrap();

		if path.is_file() {
			zip.start_file(name, options)?;
			let mut f = File::open(path)?;
			// TODO - Use BufReader and BufWriter here.
			io::copy(&mut f, &mut zip)?;
		}
	}

	zip.finish()?;
	dst_file.sync_all()?;
	Ok(())
}

/// Extract a set of files from the provided zip archive.
pub fn extract_files(from_archive: File, dest: &Path, files: &[&str]) -> io::Result<()> {
	let dest: PathBuf = PathBuf::from(dest);
	let files: Vec<_> = files.iter().map(|x| x.to_string()).collect();
	let res = thread::spawn(move || {
		let mut archive = zip_rs::ZipArchive::new(from_archive).expect("archive file exists");
		for x in files {
			let file = archive.by_name(&x).expect("file exists in archive");
			let path = dest.join(file.sanitized_name());
			let parent_dir = path.parent().expect("valid parent dir");
			fs::create_dir_all(&parent_dir).expect("create parent dir");
			let outfile = fs::File::create(&path).expect("file created");
			io::copy(&mut BufReader::new(file), &mut BufWriter::new(outfile))
				.expect("write to file");

			info!("extract_files: {:?}", path);

			// Set file permissions to "644" (Unix only).
			#[cfg(unix)]
			{
				use std::os::unix::fs::PermissionsExt;
				let mode = PermissionsExt::from_mode(0o644);
				fs::set_permissions(&path, mode).expect("set file permissions");
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
