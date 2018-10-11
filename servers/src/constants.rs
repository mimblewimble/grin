// Copyright 2018 The Grin Developers
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

//! All the constants required for all kinds of servers.
//! server-relevant constants and short functions should be kept
//! here.

/// Trigger chain compaction every 10080 blocks (i.e. one week) for FAST_SYNC_NODE
pub const COMPACTION_BLOCKS: u64 = 10_080;
