# Historical header MMR proofs (HEADER_PROOF_HIST)

Status: optional P2P extension, protocol v1. **No consensus change.**

A node that advertises `HEADER_PROOF_HIST` can serve a full header at height `H`
together with a Merkle inclusion proof of that header in the header MMR, against
the `prev_root` of an anchor header `A` chosen by the requester (any header on the
server's main chain above `H`).

This lets a light client that holds a recent header verify sampled historical
headers without downloading the whole header chain (FlyClient-style sampling),
using data every full node already has: every header commits to all previous
headers through `prev_root`.

## Capability

`Capabilities::HEADER_PROOF_HIST = 0x100` (next free bit after `PIHD_HIST`).

The bit means "this node serves header proofs". A requester does **not** need to
advertise any capability to be served.

## Messages

All integers use the existing Grin serializer (big-endian). `Hash` is 32 raw bytes.

| Id | Message          | Description |
|----|------------------|-------------|
| 31 | GetHeaderProofs  | Request proofs for up to 64 heights below an anchor. |
| 32 | HeaderProofs     | Headers with their MMR inclusion proofs. |

```text
GetHeaderProofs
  anchor:  Hash         // header on the receiver's main chain
  count:   u16          // 1..=64
  heights: [u64; count] // strictly increasing, unique, each < anchor.height

HeaderProofs
  anchor:   Hash
  mmr_size: u64         // insertion_to_pmmr_index(anchor.height)
  count:    u16
  items:    [count] {
    header:   BlockHeader       // same serialization as the Header message
    path_len: u8                // <= 64
    path:     [Hash; path_len]  // MerkleProof.path of the header leaf
  }
```

Limits: `count <= 64`, `path_len <= 64`, response at most 168,170 bytes.

The parser enforces wire bounds only. Semantic checks (non-empty, sorted, unique,
below the anchor) are done by the handler; an invalid or unanswerable request is
ignored and never leads to a ban. Requests share the per-IP budget of PIHD header
segment requests.

## MMR semantics

- Leaf position of height `h`: `pos = insertion_to_pmmr_index(h)`.
- Historical MMR size for anchor `A`: `insertion_to_pmmr_index(A.height)`; the
  server never proves against the current tip size.
- The proof root is `A.prev_root`. The server verifies every proof against it
  before answering and does not answer if the anchor is not on its main chain.

## Client verification

`grin_p2p::header_proof` provides:

- `verify_memberships(anchor, proofs)`: MMR membership only.
- `verify_header_proofs_with_pow(anchor, proofs, trusted_tail)`: membership plus
  proof of work and difficulty.

**Membership alone is not enough.** The header MMR leaf commits to the position
and the packed PoW nonces only, so fields such as `timestamp`, `output_root` or
`total_difficulty` can be altered without breaking membership. A client must use
`verify_header_proofs_with_pow`, which for every sampled height `H`:

- requires headers `H-2`, `H-1`, `H` (from the response or a trusted tail);
- checks the header version, a valid primary Cuckoo solution (which binds all
  pre-PoW fields), `prev_hash` linkage, increasing timestamps and total difficulty;
- post-HF4: checks that the claimed difficulty of `H` equals
  `next_wtema_difficulty` computed from `H-1`, `H-2`, and that the PoW reaches it.

Pre-HF4 sampled headers are rejected with an explicit error: their difficulty
(DMA, 60-header window, dual PoW) needs a separate checkpoint/window policy.

## Security notes

- A proof shows membership in the anchor's MMR, not that the anchor is on the
  highest-work chain. Anchor selection and eclipse resistance are the client's
  responsibility (for example, several peers from distinct networks).
- A header proof is not a UTXO proof; state still has to be verified separately
  (PIBD segments against `output_root`).
