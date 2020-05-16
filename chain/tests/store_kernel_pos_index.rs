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

use crate::chain::linked_list::{self, ListIndex, ListWrapper, RewindableListIndex};
use crate::chain::store::{self, ChainStore};
use crate::chain::types::CommitPos;
use crate::core::core::OutputFeatures;
use crate::util::secp::pedersen::Commitment;
use grin_chain as chain;
use grin_core as core;
use grin_store;
use grin_util as util;
mod chain_test_helper;
use self::chain_test_helper::clean_output_dir;
use crate::grin_store::Error;

#[test]
fn test_store_kernel_idx() {
	util::init_test_logger();

	let chain_dir = ".grin_idx_1";
	clean_output_dir(chain_dir);

	let commit = Commitment::from_vec(vec![]);

	let store = ChainStore::new(chain_dir).unwrap();
	let batch = store.batch().unwrap();
	let index = store::coinbase_kernel_index();

	assert_eq!(index.peek_pos(&batch, commit), Ok(None));
	assert_eq!(index.get_list(&batch, commit), Ok(None));

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 1, height: 1 }),
		Ok(()),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 1, height: 1 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Single {
			pos: CommitPos { pos: 1, height: 1 }
		})),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 2, height: 2 }),
		Ok(()),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 2, height: 2 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Multi { head: 2, tail: 1 })),
	);

	// Pos must always increase.
	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 1, height: 1 }),
		Err(Error::OtherErr("pos must be increasing".into())),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 2, height: 2 }),
		Err(Error::OtherErr("pos must be increasing".into())),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 3, height: 3 }),
		Ok(()),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 3, height: 3 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Multi { head: 3, tail: 1 })),
	);

	assert_eq!(
		index.pop_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 3, height: 3 })),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 2, height: 2 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Multi { head: 2, tail: 1 })),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 3, height: 3 }),
		Ok(()),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 3, height: 3 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Multi { head: 3, tail: 1 })),
	);

	assert_eq!(
		index.pop_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 3, height: 3 })),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 2, height: 2 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Multi { head: 2, tail: 1 })),
	);

	assert_eq!(
		index.pop_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 2, height: 2 })),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 1, height: 1 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Single {
			pos: CommitPos { pos: 1, height: 1 }
		})),
	);

	assert_eq!(
		index.pop_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 1, height: 1 })),
	);

	assert_eq!(index.peek_pos(&batch, commit), Ok(None));
	assert_eq!(index.get_list(&batch, commit), Ok(None));

	// Cleanup chain directory
	clean_output_dir(chain_dir);
}

#[test]
fn test_store_kernel_idx_pop_back() {
	util::init_test_logger();

	let chain_dir = ".grin_idx_2";
	clean_output_dir(chain_dir);

	let commit = Commitment::from_vec(vec![]);

	let store = ChainStore::new(chain_dir).unwrap();
	let batch = store.batch().unwrap();
	let index = store::coinbase_kernel_index();

	assert_eq!(index.peek_pos(&batch, commit), Ok(None));
	assert_eq!(index.get_list(&batch, commit), Ok(None));

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 1, height: 1 }),
		Ok(()),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 1, height: 1 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Single {
			pos: CommitPos { pos: 1, height: 1 }
		})),
	);

	assert_eq!(
		index.pop_pos_back(&batch, commit),
		Ok(Some(CommitPos { pos: 1, height: 1 })),
	);

	assert_eq!(index.peek_pos(&batch, commit), Ok(None));
	assert_eq!(index.get_list(&batch, commit), Ok(None));

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 1, height: 1 }),
		Ok(()),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 2, height: 2 }),
		Ok(()),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 3, height: 3 }),
		Ok(()),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 3, height: 3 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Multi { head: 3, tail: 1 })),
	);

	assert_eq!(
		index.pop_pos_back(&batch, commit),
		Ok(Some(CommitPos { pos: 1, height: 1 })),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 3, height: 3 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Multi { head: 3, tail: 2 })),
	);

	assert_eq!(
		index.pop_pos_back(&batch, commit),
		Ok(Some(CommitPos { pos: 2, height: 2 })),
	);

	assert_eq!(
		index.peek_pos(&batch, commit),
		Ok(Some(CommitPos { pos: 3, height: 3 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Single {
			pos: CommitPos { pos: 3, height: 3 }
		})),
	);

	assert_eq!(
		index.pop_pos_back(&batch, commit),
		Ok(Some(CommitPos { pos: 3, height: 3 })),
	);

	assert_eq!(index.peek_pos(&batch, commit), Ok(None));
	assert_eq!(index.get_list(&batch, commit), Ok(None));

	clean_output_dir(chain_dir);
}

#[test]
fn test_store_kernel_idx_rewind() {
	util::init_test_logger();

	let chain_dir = ".grin_idx_3";
	clean_output_dir(chain_dir);

	let commit = Commitment::from_vec(vec![]);

	let store = ChainStore::new(chain_dir).unwrap();
	let batch = store.batch().unwrap();
	let index = store::coinbase_kernel_index();

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 1, height: 1 }),
		Ok(()),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 2, height: 2 }),
		Ok(()),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 3, height: 3 }),
		Ok(()),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Multi { head: 3, tail: 1 })),
	);

	assert_eq!(index.rewind(&batch, commit, 1), Ok(()),);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Single {
			pos: CommitPos { pos: 1, height: 1 }
		})),
	);

	// Check we can safely noop rewind.
	assert_eq!(index.rewind(&batch, commit, 2), Ok(()),);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Single {
			pos: CommitPos { pos: 1, height: 1 }
		})),
	);

	assert_eq!(index.rewind(&batch, commit, 1), Ok(()),);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Single {
			pos: CommitPos { pos: 1, height: 1 }
		})),
	);

	// Check we can rewind back to 0.
	assert_eq!(index.rewind(&batch, commit, 0), Ok(()),);

	assert_eq!(index.get_list(&batch, commit), Ok(None),);

	assert_eq!(index.rewind(&batch, commit, 0), Ok(()),);

	// Now check we can rewind past the end of a list safely.

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 1, height: 1 }),
		Ok(()),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 2, height: 2 }),
		Ok(()),
	);

	assert_eq!(
		index.push_pos(&batch, commit, CommitPos { pos: 3, height: 3 }),
		Ok(()),
	);

	assert_eq!(
		index.pop_pos_back(&batch, commit),
		Ok(Some(CommitPos { pos: 1, height: 1 })),
	);

	assert_eq!(
		index.get_list(&batch, commit),
		Ok(Some(ListWrapper::Multi { head: 3, tail: 2 })),
	);

	assert_eq!(index.rewind(&batch, commit, 1), Ok(()),);

	assert_eq!(index.get_list(&batch, commit), Ok(None),);

	clean_output_dir(chain_dir);
}
