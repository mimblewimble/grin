// Copyright 2017 The Grin Developers
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

extern crate env_logger;
extern crate grin_core as core;
extern crate grin_store as store;
extern crate time;

use std::fs;

use core::ser::*;
use core::core::pmmr::{PMMR, Summable, HashSum, Backend};
use core::core::hash::Hashed;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct TestElem([u32; 4]);
impl Summable for TestElem {
	type Sum = u64;
	fn sum(&self) -> u64 {
		// sums are not allowed to overflow, so we use this simple
		// non-injective "sum" function that will still be homomorphic
		self.0[0] as u64 * 0x1000 + self.0[1] as u64 * 0x100 + self.0[2] as u64 * 0x10 +
			self.0[3] as u64
	}
	fn sum_len() -> usize {
		8
	}
}

impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		try!(writer.write_u32(self.0[0]));
		try!(writer.write_u32(self.0[1]));
		try!(writer.write_u32(self.0[2]));
		writer.write_u32(self.0[3])
	}
}

#[test]
fn sumtree_append() {
	let _ = env_logger::init();

	let data_dir = format!("./{}", time::get_time().sec);
	fs::create_dir_all(data_dir.clone()).unwrap();

	let elems = vec![
		TestElem([0, 0, 0, 1]),
		TestElem([0, 0, 0, 2]),
		TestElem([0, 0, 0, 3]),
		TestElem([0, 0, 0, 4]),
		TestElem([0, 0, 0, 5]),
		TestElem([0, 0, 0, 6]),
		TestElem([0, 0, 0, 7]),
		TestElem([0, 0, 0, 8]),
		TestElem([1, 0, 0, 0]),
	];

	let mut mmr_size: u64;
	let mut backend = store::sumtree::PMMRBackend::new(data_dir).unwrap();
	{
		let mut pmmr = PMMR::new(&mut backend);
		for elem in &elems[0..4] {
			pmmr.push(elem.clone());
		}
		mmr_size = pmmr.unpruned_size();
	}
	backend.sync().unwrap();

	{
		let mut pmmr = PMMR::at(&mut backend, mmr_size);
		for elem in &elems[4..elems.len()] {
			pmmr.push(elem.clone());
		}
		mmr_size = pmmr.unpruned_size();
	}
	backend.sync().unwrap();

	let hash = Hashed::hash(&elems[0].clone());
	let sum = elems[0].sum();
	let node_hash = (1 as u64, &sum, hash).hash();
	assert_eq!(
		backend.get(1),
		Some(HashSum {
			hash: node_hash,
			sum: sum,
		})
	);

	let sum2 = HashSum::from_summable(1, &elems[0]) + HashSum::from_summable(2, &elems[1]);
	let sum4 = sum2 + (HashSum::from_summable(4, &elems[2]) + HashSum::from_summable(5, &elems[3]));
	let sum8 = sum4 +
		((HashSum::from_summable(8, &elems[4]) + HashSum::from_summable(9, &elems[5])) +
			 (HashSum::from_summable(11, &elems[6]) + HashSum::from_summable(12, &elems[7])));
	let sum9 = sum8 + HashSum::from_summable(16, &elems[8]);

	{
		let pmmr = PMMR::at(&mut backend, mmr_size);
		assert_eq!(pmmr.root(), sum9);
	}
}
