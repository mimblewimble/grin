// Copyright 2022 The Grin Developers
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

//! Set of static definitions for all parameters related to PIBD and Desegmentation
//! Note these are for experimentation via compilation, not meant to be exposed as
//! configuration parameters anywhere

/// Bitmap segment height assumed for requests and segment calculation
pub const BITMAP_SEGMENT_HEIGHT: u8 = 9;

/// Output segment height assumed for requests and segment calculation
pub const OUTPUT_SEGMENT_HEIGHT: u8 = 11;

/// Rangeproof segment height assumed for requests and segment calculation
pub const RANGEPROOF_SEGMENT_HEIGHT: u8 = 11;

/// Kernel segment height assumed for requests and segment calculation
pub const KERNEL_SEGMENT_HEIGHT: u8 = 11;

/// Maximum number of received segments to cache (across all trees) before we stop requesting others
pub const MAX_CACHED_SEGMENTS: usize = 15;

/// How long the state sync should wait after requesting a segment from a peer before
/// deciding the segment isn't going to arrive. The syncer will then re-request the segment
pub const SEGMENT_REQUEST_TIMEOUT_SECS: i64 = 60;

/// Number of simultaneous requests for segments we should make. Note this is currently
/// divisible by 3 to try and evenly spread requests amount the 3 main MMRs (Bitmap segments
/// will always be requested first)
pub const SEGMENT_REQUEST_COUNT: usize = 15;

/// If the syncer hasn't seen a max work peer that supports PIBD in this number of seconds
/// give up and revert back to the txhashset.zip download method
pub const TXHASHSET_ZIP_FALLBACK_TIME_SECS: i64 = 60;
