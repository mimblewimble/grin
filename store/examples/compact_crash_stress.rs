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

//! Stress harness for PMMR compaction on weak hardware and crash recovery.
//!
//! Used by `etc/qemu-compact-stress/run.sh` to simulate a low-end VPS
//! (slow CPU via QEMU, tight RAM) and hard-kill mid-compaction.
//!
//! **Testnet is usually the easiest real-world repro** — often heavier than
//! mainnet even on a desktop: more spent/UTXO churn, and the first compact
//! after PIBD rewrites almost the entire hash/data files against a huge prune
//! set. Long-running mainnet nodes compact incrementally (smaller deltas).
//! Use a high `prune_pct` (runner `--testnet-like`) to model that case.
//!
//! Subcommands:
//! - `prepare <dir> <leaves> [prune_pct]` — build PMMR, prune pct% of leaves
//! - `compact <dir>` — run `check_compact`, print duration (and a 2nd pass)
//! - `verify <dir>` — open and check root matches `root.txt`
//! - `kill-test <dir> <leaves> <phase> [prune_pct]` — SIGKILL mid-compact
//!
//! Env (also used by check_compact test hooks):
//! - `GRIN_COMPACT_READY_FILE`, `GRIN_COMPACT_PAUSE_PHASE`,
//!   `GRIN_COMPACT_CRASH_PAUSE_MS`, `GRIN_COMPACT_PRUNE_PCT`

use croaring::Bitmap;
use grin_core::core::hash::{DefaultHashable, Hash};
use grin_core::core::pmmr::{ReadablePMMR, PMMR};
use grin_core::ser::{Error, PMMRable, ProtocolVersion, Readable, Reader, Writeable, Writer};
use grin_store::pmmr::PMMRBackend;
use grin_util::ToHex;
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct TestElem(u32);

impl DefaultHashable for TestElem {}

impl PMMRable for TestElem {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		*self
	}

	fn elmt_size() -> Option<u16> {
		Some(4)
	}
}

impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_u32(self.0)
	}
}

impl Readable for TestElem {
	fn read<R: Reader>(reader: &mut R) -> Result<TestElem, Error> {
		Ok(TestElem(reader.read_u32()?))
	}
}

fn usage() -> ! {
	eprintln!(
		"usage:
  compact_crash_stress prepare <data_dir> <n_leaves> [prune_pct]
  compact_crash_stress compact <data_dir>
  compact_crash_stress verify  <data_dir>
  compact_crash_stress kill-test <data_dir> <n_leaves> <phase> [prune_pct]

prune_pct: 0-99 percent of leaves to prune before compact (default 50).
  Testnet-like first compact after heavy spend/PIBD: try 90-95.

phases for kill-test: hash_tmp | data_tmp | journal
"
	);
	std::process::exit(2);
}

fn parse_prune_pct(arg: Option<String>) -> u32 {
	let pct = arg
		.or_else(|| env::var("GRIN_COMPACT_PRUNE_PCT").ok())
		.map(|s| s.parse::<u32>().expect("prune_pct u32"))
		.unwrap_or(50);
	assert!(
		pct < 100,
		"prune_pct must be 0..99 (keep at least some UTXOs)"
	);
	pct
}

fn root_path(dir: &Path) -> PathBuf {
	dir.join("root.txt")
}
fn size_path(dir: &Path) -> PathBuf {
	dir.join("mmr_size.txt")
}

fn write_meta(dir: &Path, root: Hash, mmr_size: u64) {
	// Full 32-byte hex (Display truncates for logging).
	fs::write(root_path(dir), root.as_bytes().to_hex()).unwrap();
	fs::write(size_path(dir), format!("{}", mmr_size)).unwrap();
}

fn read_meta(dir: &Path) -> (Hash, u64) {
	let mut root_s = String::new();
	File::open(root_path(dir))
		.unwrap()
		.read_to_string(&mut root_s)
		.unwrap();
	let root = Hash::from_hex(root_s.trim()).expect("valid root hex");
	let size: u64 = fs::read_to_string(size_path(dir))
		.unwrap()
		.trim()
		.parse()
		.unwrap();
	(root, size)
}

fn open_backend(dir: &Path) -> PMMRBackend<TestElem> {
	PMMRBackend::new(dir, true, ProtocolVersion(1), None).expect("open backend")
}

/// Build a prunable PMMR with `n_leaves`, prune `prune_pct`% of leaves
/// (stable root), sync, and persist root/size metadata.
///
/// High prune_pct (90–95) models testnet / post-PIBD first compact: a large
/// prune set and a full hash+data file rewrite, which is much heavier than
/// the small deltas a continuously compacting mainnet node typically sees.
fn cmd_prepare(dir: &Path, n_leaves: u32, prune_pct: u32) {
	if dir.exists() {
		fs::remove_dir_all(dir).unwrap();
	}
	fs::create_dir_all(dir).unwrap();

	let t0 = Instant::now();
	let mut backend = open_backend(dir);
	let mut mmr = PMMR::new(&mut backend);
	for i in 0..n_leaves {
		mmr.push(&TestElem(i)).unwrap();
	}
	let mmr_size = mmr.unpruned_size();
	let root = mmr.root().unwrap();
	drop(mmr);
	backend.sync().unwrap();

	// Keep every Nth leaf so (prune_pct)% are removed. N = 100 / (100 - pct).
	// e.g. prune_pct=50 → keep every 2nd; prune_pct=90 → keep every 10th.
	let keep_every = (100u32 / (100 - prune_pct)).max(1);
	let mut pruned = 0u32;
	{
		let mut mmr = PMMR::at(&mut backend, mmr_size);
		for i in 0..n_leaves {
			if i % keep_every != 0 {
				let pos0 = grin_core::core::pmmr::insertion_to_pmmr_index(i as u64);
				if mmr.prune(pos0).unwrap_or(false) {
					pruned += 1;
				}
			}
		}
	}
	backend.sync().unwrap();
	write_meta(dir, root, mmr_size);

	let hash_bytes = fs::metadata(dir.join("pmmr_hash.bin"))
		.map(|m| m.len())
		.unwrap_or(0);
	let data_bytes = fs::metadata(dir.join("pmmr_data.bin"))
		.map(|m| m.len())
		.unwrap_or(0);
	println!(
		"prepare: leaves={} pruned={} (~{}%, keep_every={}) mmr_size={} hash_bytes={} data_bytes={} root={} took={:.2}s",
		n_leaves,
		pruned,
		prune_pct,
		keep_every,
		mmr_size,
		hash_bytes,
		data_bytes,
		root,
		t0.elapsed().as_secs_f64()
	);
}

fn cmd_compact(dir: &Path) {
	let (expected_root, mmr_size) = read_meta(dir);
	let mut backend = open_backend(dir);

	// First compact: full rewrite against the accumulated prune set
	// (testnet / post-PIBD worst case).
	let t0 = Instant::now();
	backend
		.check_compact(mmr_size, &Bitmap::new())
		.expect("check_compact");
	backend.sync().unwrap();
	let first = t0.elapsed();

	// Second compact with no new prunes: should be much cheaper — closer to
	// a long-running mainnet node that only rewrites a small delta.
	let t1 = Instant::now();
	backend
		.check_compact(mmr_size, &Bitmap::new())
		.expect("check_compact second");
	backend.sync().unwrap();
	let second = t1.elapsed();

	let pmmr = PMMR::at(&mut backend, mmr_size);
	let root = pmmr.root().unwrap();
	assert_eq!(root, expected_root, "root must be unchanged by compact");
	println!(
		"compact: ok root={} first={:.3}s ({:.0} ms) second={:.3}s ({:.0} ms) ratio={:.1}x",
		root,
		first.as_secs_f64(),
		first.as_secs_f64() * 1000.0,
		second.as_secs_f64(),
		second.as_secs_f64() * 1000.0,
		if second.as_secs_f64() > 0.0 {
			first.as_secs_f64() / second.as_secs_f64()
		} else {
			0.0
		}
	);
}

fn cmd_verify(dir: &Path) {
	let (expected_root, mmr_size) = read_meta(dir);
	let mut backend = open_backend(dir);
	let pmmr = PMMR::at(&mut backend, mmr_size);
	let root = pmmr.root().unwrap();
	assert_eq!(
		root, expected_root,
		"root mismatch after open/recovery: got {} want {}",
		root, expected_root
	);
	// Spot-check: leaf insertion index 0 is always kept by prepare (keep_every).
	assert!(
		pmmr.n_unpruned_leaves() > 0,
		"expected unpruned leaves after compact/recovery"
	);
	assert!(
		pmmr.get_data(0).is_some(),
		"expected first leaf (insertion 0) still readable"
	);
	assert!(
		!dir.join("pmmr_compact.journal").exists(),
		"journal must not remain after successful open"
	);
	println!("verify: ok root={}", root);
}

fn wait_for_phase(ready_file: &Path, phase: &str, timeout: Duration) -> bool {
	let start = Instant::now();
	while start.elapsed() < timeout {
		if let Ok(contents) = fs::read_to_string(ready_file) {
			if contents.trim() == phase {
				return true;
			}
		}
		thread::sleep(Duration::from_millis(20));
	}
	false
}

/// Prepare dataset, spawn `compact` child paused at `phase`, SIGKILL it, verify.
fn cmd_kill_test(dir: &Path, n_leaves: u32, phase: &str, prune_pct: u32) {
	match phase {
		"hash_tmp" | "data_tmp" | "journal" => {}
		_ => {
			eprintln!("unknown phase '{}'", phase);
			usage();
		}
	}

	cmd_prepare(dir, n_leaves, prune_pct);
	let (expected_root, _) = read_meta(dir);

	let ready_file = dir.join("compact_ready");
	let _ = fs::remove_file(&ready_file);

	let exe = env::current_exe().expect("current_exe");
	let mut child = Command::new(&exe)
		.arg("compact")
		.arg(dir)
		.env("GRIN_COMPACT_READY_FILE", &ready_file)
		.env("GRIN_COMPACT_PAUSE_PHASE", phase)
		.env("GRIN_COMPACT_CRASH_PAUSE_MS", "30000")
		.stdout(Stdio::inherit())
		.stderr(Stdio::inherit())
		.spawn()
		.expect("spawn compact child");

	println!(
		"kill-test: child pid={} waiting for phase '{}' ...",
		child.id(),
		phase
	);
	let saw = wait_for_phase(&ready_file, phase, Duration::from_secs(600));
	if !saw {
		let _ = child.kill();
		let _ = child.wait();
		panic!(
			"timed out waiting for compact phase '{}' (ready file: {:?})",
			phase, ready_file
		);
	}

	// Hard kill: no graceful shutdown (simulates OOM killer / kill -9 / power loss).
	#[cfg(unix)]
	{
		let pid = child.id() as i32;
		let rc = unsafe { libc::kill(pid, libc::SIGKILL) };
		assert_eq!(rc, 0, "SIGKILL failed");
	}
	#[cfg(not(unix))]
	{
		child.kill().expect("kill child");
	}
	let status = child.wait().expect("wait child");
	println!(
		"kill-test: child exited {:?} after SIGKILL at '{}'",
		status, phase
	);

	// Parent re-opens: recovery must leave a consistent store with the same root.
	cmd_verify(dir);
	let (root, _) = read_meta(dir);
	assert_eq!(root, expected_root);
	println!(
		"kill-test: PASS phase={} root recovered={}",
		phase, expected_root
	);
}

fn main() {
	let mut args = env::args().skip(1);
	let cmd = args.next().unwrap_or_else(|| usage());
	match cmd.as_str() {
		"prepare" => {
			let dir = PathBuf::from(args.next().unwrap_or_else(|| usage()));
			let n: u32 = args
				.next()
				.unwrap_or_else(|| usage())
				.parse()
				.expect("n_leaves");
			let prune_pct = parse_prune_pct(args.next());
			cmd_prepare(&dir, n, prune_pct);
		}
		"compact" => {
			let dir = PathBuf::from(args.next().unwrap_or_else(|| usage()));
			cmd_compact(&dir);
		}
		"verify" => {
			let dir = PathBuf::from(args.next().unwrap_or_else(|| usage()));
			cmd_verify(&dir);
		}
		"kill-test" => {
			let dir = PathBuf::from(args.next().unwrap_or_else(|| usage()));
			let n: u32 = args
				.next()
				.unwrap_or_else(|| usage())
				.parse()
				.expect("n_leaves");
			let phase = args.next().unwrap_or_else(|| usage());
			let prune_pct = parse_prune_pct(args.next());
			cmd_kill_test(&dir, n, &phase, prune_pct);
		}
		_ => usage(),
	}
}
