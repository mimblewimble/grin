// Copyright 2020 The Grin Developers
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

//! Utility structs to handle the 3 hashtrees (output, range proof,
//! kernel) more conveniently and transactionally.

mod bitmap_accumulator;
mod rewindable_kernel_view;
mod rewindable_utxo_bitmap;
mod txhashset;
mod utxo_view;

pub use self::bitmap_accumulator::*;
pub use self::rewindable_kernel_view::*;
pub use self::rewindable_utxo_bitmap::*;
pub use self::txhashset::*;
pub use self::utxo_view::*;
