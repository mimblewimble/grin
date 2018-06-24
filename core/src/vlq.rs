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

/// Variable Length Quantity encoding. See:
/// https://en.wikipedia.org/wiki/Variable-length_quantity
use ser::{Error, Reader, Writer};

const MASK: u8 = 0b01111111;
const HIGH_BIT: u8 = 0b10000000;

/// Read a provided number of VLQ encoded numbers from the provided
/// reader and returns them as u64.
pub fn read(n: usize, reader: &mut Reader) -> Result<Vec<u64>, Error> {
	let mut qties = Vec::with_capacity(n);
	for m in 0..n {
		let mut qty: u64 = 0;
		for n in 0..8 {
			let b = reader.read_u8()?;
			qty += ((b & MASK) as u64) << (n * 7);
			println!("    {} {}", b, qty);
			if (b & HIGH_BIT) == 0 {
				break;
			}
		}
		println!("{}+ {}", m, qty);
		qties.push(qty);
	}
	Ok(qties)
}

pub fn write<W>(qties: Vec<u64>, writer: &mut W) -> Result<(), Error>
where
	W: Writer,
{
	for mut qty in qties {
		loop {
			let mut b = qty & (MASK as u64);
			qty >>= 7;
			if qty > 0 {
				b |= HIGH_BIT as u64;
			}
			writer.write_u8(b as u8)?;
			if qty == 0 {
				break;
			}
		}
	}
	Ok(())
}

#[cfg(test)]
mod test {
	use super::*;
	use ser::{self, Readable, Writeable};

	struct MyVec {
		values: Vec<u64>,
	}
	impl Readable for MyVec {
		fn read(reader: &mut Reader) -> Result<MyVec, Error> {
			Ok(MyVec {
				values: read(10, reader)?,
			})
		}
	}
	impl Writeable for MyVec {
		fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
			write(self.values.clone(), writer)
		}
	}

	#[test]
	fn roundtrip_vlqs() {
		let myvec = MyVec {
			values: vec![1000, 100, 127, 128, 65535, 65536, 1000000, 0, 1, 255],
		};
		let vlqs = ser::ser_vec(&myvec).unwrap();
		assert_eq!(vlqs[0], 232);
		assert_eq!(vlqs[1], 7);
		assert_eq!(vlqs[2], 100);
		let myvec2: MyVec = ser::deserialize(&mut &vlqs[..]).unwrap();
		for n in 0..10 {
			assert_eq!(myvec.values[n], myvec2.values[n]);
		}
	}
}
