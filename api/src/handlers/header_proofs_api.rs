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

//! Historical header MMR inclusion proofs for remote-node wallets.

use super::utils::w;
use crate::chain;
use crate::core::core::hash::Hash;
use crate::core::core::header_proof::Error as ProofError;
use crate::rest::*;
use crate::types::HeaderProofsPrintable;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Weak;

/// At most this many proof builds run at once on a node. Extra requests are
/// refused at once rather than queued.
pub const MAX_CONCURRENT_HEADER_PROOFS: usize = 2;

/// Context headers per sampled height when the caller gives none: enough for
/// the exact difficulty check after HF4.
pub const DEFAULT_HEADER_PROOF_CONTEXT: u64 = 2;

/// Slots shared by all header proof requests of this node.
pub static HEADER_PROOF_SLOTS: ProofSlots = ProofSlots::new(MAX_CONCURRENT_HEADER_PROOFS);

/// A counter of proof builds in progress with a fixed maximum.
pub struct ProofSlots {
	used: AtomicUsize,
	max: usize,
}

/// A taken slot; dropping it frees the slot.
pub struct ProofSlot<'a>(&'a ProofSlots);

impl ProofSlots {
	/// Slots for at most `max` builds at once.
	pub const fn new(max: usize) -> ProofSlots {
		ProofSlots {
			used: AtomicUsize::new(0),
			max,
		}
	}

	/// Takes a slot if one is free.
	pub fn try_take(&self) -> Option<ProofSlot<'_>> {
		self.used
			.fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
				if used < self.max {
					Some(used + 1)
				} else {
					None
				}
			})
			.ok()
			.map(|_| ProofSlot(self))
	}
}

impl Drop for ProofSlot<'_> {
	fn drop(&mut self) {
		self.0.used.fetch_sub(1, Ordering::AcqRel);
	}
}

/// Serves `get_header_proofs`.
pub struct HeaderProofsHandler {
	pub chain: Weak<chain::Chain>,
	pub slots: &'static ProofSlots,
}

impl HeaderProofsHandler {
	pub fn get_header_proofs(
		&self,
		anchor: &str,
		heights: &[u64],
		context: Option<u64>,
	) -> Result<HeaderProofsPrintable, Error> {
		let anchor = Hash::from_hex(anchor)
			.map_err(|_| Error::Argument(format!("Not a valid hash: {}", anchor)))?;
		let context = usize::try_from(context.unwrap_or(DEFAULT_HEADER_PROOF_CONTEXT))
			.map_err(|_| Error::Argument("context is too large".to_owned()))?;
		let _slot = self.slots.try_take().ok_or_else(|| {
			Error::RequestError("too many header proof requests at once, retry later".to_owned())
		})?;
		let proofs = w(&self.chain)?
			.header_proofs(anchor, heights, context)
			.map_err(|e| match e {
				chain::Error::HeaderProof(ProofError::UnknownAnchor(_)) => Error::NotFound,
				chain::Error::HeaderProof(
					e @ (ProofError::ItemCount(_)
					| ProofError::ContextTooLarge(_)
					| ProofError::TooManyHeaders(_)
					| ProofError::NotIncreasing(_, _)
					| ProofError::NotBelowAnchor(_, _)
					| ProofError::NoRoomForContext(_, _)
					| ProofError::AnchorNotOnMainChain(_)),
				) => Error::Argument(e.to_string()),
				e => Error::Internal(e.to_string()),
			})?;
		HeaderProofsPrintable::from_proofs(&proofs)
			.map_err(|e| Error::Internal(format!("cannot serialize header proofs: {}", e)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn slots_refuse_beyond_the_maximum() {
		let slots = ProofSlots::new(2);
		let first = slots.try_take().expect("first slot");
		let second = slots.try_take().expect("second slot");
		assert!(slots.try_take().is_none());
		drop(first);
		let third = slots.try_take().expect("a freed slot");
		assert!(slots.try_take().is_none());
		drop(second);
		drop(third);
		assert_eq!(slots.used.load(Ordering::Acquire), 0);
	}

	#[test]
	fn a_busy_node_refuses_before_touching_the_chain() {
		static BUSY: ProofSlots = ProofSlots::new(1);
		let _held = BUSY.try_take().expect("slot");
		let handler = HeaderProofsHandler {
			chain: Weak::new(),
			slots: &BUSY,
		};
		let anchor = "0".repeat(64);
		assert!(matches!(
			handler.get_header_proofs(&anchor, &[1], Some(0)),
			Err(Error::RequestError(_))
		));
	}

	#[test]
	fn a_bad_anchor_is_an_argument_error() {
		let handler = HeaderProofsHandler {
			chain: Weak::new(),
			slots: &HEADER_PROOF_SLOTS,
		};
		assert!(matches!(
			handler.get_header_proofs("not hex", &[1], None),
			Err(Error::Argument(_))
		));
	}
}
