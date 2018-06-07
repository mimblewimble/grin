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

extern crate croaring;
use croaring::Bitmap;


#[test]
fn test_a_small_bitmap() {
	let bitmap: Bitmap = vec![1, 99, 1_000].into_iter().collect();
	let serialized_buffer = bitmap.serialize();

	// we can store 3 pos in a roaring bitmap in 22 bytes
	// this is compared to storing them as a vec of u64 values which would be 8 * 3 = 32 bytes
	assert_eq!(serialized_buffer.len(), 22);
}

#[test]
fn test_a_big_bitmap() {
	let mut bitmap: Bitmap = (1..1_000).collect();
	let serialized_buffer = bitmap.serialize();

	// we can also store 1,000 pos in 2,014 bytes
	// a vec of u64s here would be 8,000 bytes
	assert_eq!(serialized_buffer.len(), 2014);

	// but note that we can optimize this heavily to get down to 15 bytes...
	assert!(bitmap.run_optimize());
	let serialized_buffer = bitmap.serialize();
	assert_eq!(serialized_buffer.len(), 15);
}
