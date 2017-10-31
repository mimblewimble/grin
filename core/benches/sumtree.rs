// Copyright 2016 The Grin Developers
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

#![feature(test)]

extern crate grin_core as core;
extern crate rand;
extern crate test;

use rand::Rng;
use test::Bencher;

use core::core::sumtree::{self, SumTree, Summable};
use core::ser::{Error, Writeable, Writer};

#[derive(Copy, Clone, Debug)]
struct TestElem([u32; 4]);
impl Summable for TestElem {
	type Sum = u64;
	fn sum(&self) -> u64 {
		// sums are not allowed to overflow, so we use this simple
  // non-injective "sum" function that will still be homomorphic
		self.0[0] as u64 * 0x1000 + self.0[1] as u64 * 0x100 + self.0[2] as u64 * 0x10
			+ self.0[3] as u64
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

#[bench]
fn bench_small_tree(b: &mut Bencher) {
	let mut rng = rand::thread_rng();
	b.iter(|| {
		let mut big_tree = SumTree::new();
		for i in 0..1000 {
			// To avoid RNG overflow we generate random elements that are small.
   // Though to avoid repeat elements they have to be reasonably big.
			let new_elem;
			let word1 = rng.gen::<u16>() as u32;
			let word2 = rng.gen::<u16>() as u32;
			if rng.gen() {
				if rng.gen() {
					new_elem = TestElem([word1, word2, 0, 0]);
				} else {
					new_elem = TestElem([word1, 0, word2, 0]);
				}
			} else {
				if rng.gen() {
					new_elem = TestElem([0, word1, 0, word2]);
				} else {
					new_elem = TestElem([0, 0, word1, word2]);
				}
			}

			big_tree.push(new_elem);
		}
	});
}
