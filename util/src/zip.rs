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

use std::fs::{self, File};
/// Wrappers around the `zip-rs` library to compress and decompress zip
/// bzip2 archives.
use std::io;
use std::path::Path;
use walkdir::WalkDir;

use zip_rs;
use zip_rs::result::{ZipError, ZipResult};
use zip_rs::write::FileOptions;

/// Compress a source directory recursively into a zip file using the
/// bzip2 format. Permissions are set to 644 by default to avoid any
/// unwanted execution bits.
pub fn compress(src_dir: &Path, dst_file: &File) -> ZipResult<()> {
	if !Path::new(src_dir).is_dir() {
		return Err(ZipError::Io(io::Error::new(
			io::ErrorKind::Other,
			"Source must be a directory.",
		)));
	}

	let options = FileOptions::default()
		.compression_method(zip_rs::CompressionMethod::Bzip2)
		.unix_permissions(0o644);

	let mut zip = zip_rs::ZipWriter::new(dst_file);
	let walkdir = WalkDir::new(src_dir.to_str().unwrap());
	let it = walkdir.into_iter();

	for dent in it.filter_map(|e| e.ok()) {
		let path = dent.path();
		let name = path.strip_prefix(Path::new(src_dir))
			.unwrap()
			.to_str()
			.unwrap();

		if path.is_file() {
			zip.start_file(name, options)?;
			let mut f = File::open(path)?;
			io::copy(&mut f, &mut zip)?;
		}
	}

	zip.finish()?;
	dst_file.sync_all()?;
	Ok(())
}

/// Decompress a source file into the provided destination path.
pub fn decompress<R>(src_file: R, dest: &Path) -> ZipResult<()>
where
	R: io::Read + io::Seek,
{
	let mut archive = zip_rs::ZipArchive::new(src_file)?;

	for i in 0..archive.len() {
		let mut file = archive.by_index(i)?;
		let file_path = dest.join(file.name());

		if (&*file.name()).ends_with('/') {
			fs::create_dir_all(&file_path)?;
		} else {
			if let Some(p) = file_path.parent() {
				if !p.exists() {
					fs::create_dir_all(&p)?;
				}
			}
			let mut outfile = fs::File::create(&file_path)?;
			io::copy(&mut file, &mut outfile)?;
		}

		// Get and Set permissions
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			if let Some(mode) = file.unix_mode() {
				fs::set_permissions(
					&file_path.to_str().unwrap(),
					PermissionsExt::from_mode(mode),
				)?;
			}
		}
	}
	Ok(())
}