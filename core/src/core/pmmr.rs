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

//! Persistent and prunable Merkle Mountain Range implementation. For a high
//! level description of MMRs, see:
//!
//! https://github.
//! com/opentimestamps/opentimestamps-server/blob/master/doc/merkle-mountain-range.
//! md
//!
//! This implementation is built in two major parts:
//!
//! 1. A set of low-level functions that allow navigation within an arbitrary
//! sized binary tree traversed in postorder. To realize why this us useful,
//! we start with the standard height sequence in a MMR: 0010012001... This is
//! in fact identical to the postorder traversal (left-right-top) of a binary
//! tree. In addition postorder traversal is independent of the height of the
//! tree. This allows us, with a few primitive, to get the height of any node
//! in the MMR from its position in the sequence, as well as calculate the
//! position of siblings, parents, etc. As all those functions only rely on
//! binary operations, they're extremely fast.
//! 2. The implementation of a prunable MMR tree using the above. Each leaf
//! is required to be Writeable (which implements Hashed). Tree roots can be
//! trivially and efficiently calculated without materializing the full tree.
//! The underlying Hashes are stored in a Backend implementation that can
//! either be a simple Vec or a database.

mod backend;
mod pmmr;
mod readonly_pmmr;
mod rewindable_pmmr;
pub mod segment;
mod vec_backend;

pub use self::backend::*;
pub use self::pmmr::*;
pub use self::readonly_pmmr::*;
pub use self::rewindable_pmmr::*;
pub use self::vec_backend::*;
