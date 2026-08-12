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

//! Multi-node integration tests for the Grin node.
//!
//! These tests re-introduce node-only coverage that lived in `servers/tests`
//! before the wallet was extracted (see mimblewimble/grin#2957). Wallet-coupled
//! flows remain in the `grin-wallet` repository.
