// Copyright 2026 The Grin Developers
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

//! Static definitions for PIHD sync and header-sync fallback parameters
//! Note these are for experimentation via compilation, not meant to be exposed as
//! configuration parameters anywhere

/// Maximum number of in-flight header segment requests.
pub const MAX_IN_FLIGHT_SEGMENTS: usize = 8;

/// Maximum number of header segment requests to send per sync tick.
pub const MAX_REQUESTS_PER_TICK: usize = 8;

/// Maximum number of in-flight header segment requests per peer.
pub const MAX_IN_FLIGHT_SEGMENTS_PER_PEER: usize = 2;

/// Number of seconds before treating a PIHD header segment request as timed out.
pub const PIHD_HEADER_REQUEST_TIMEOUT_SECS: i64 = 20;

/// Number of seconds before treating a legacy header request as timed out.
pub const LEGACY_HEADER_REQUEST_TIMEOUT_SECS: i64 = 10;

/// Number of failed segment requests before falling back temporarily.
pub const MAX_TIMED_OUT_SEGMENTS: usize = 3;

/// Number of seconds PIHD is disabled after repeated segment failures.
pub const DISABLE_SECS: i64 = 120;

/// Number of seconds before retrying a peer after a timed-out PIHD request.
pub const PEER_TIMEOUT_COOLDOWN_SECS: i64 = 30;

/// Number of PIHD peer failures before temporarily blocking a peer.
pub const MAX_PEER_FAILURES_BEFORE_BLOCK: usize = 3;

/// Number of seconds for retaining PIHD peer failures.
pub const PEER_FAILURE_WINDOW_SECS: i64 = 120;

/// Number of seconds PIHD may stall before falling back to legacy header sync.
pub const STALL_FALLBACK_SECS: i64 = 120;
