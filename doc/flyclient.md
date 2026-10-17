# FlyClient for Grin — Design Decision (Phase 0)

**Status:** Path A recommended (no consensus change for first implementation)  
**Issues:** [#1555](https://github.com/mimblewimble/grin/issues/1555), [#3479](https://github.com/mimblewimble/grin/issues/3479)  
**Date:** 2026-07-11

## Background

[FlyClient](https://eprint.iacr.org/2019/226) lets a light client verify the most-work PoW chain by downloading **O(λ log n)** headers (not the full chain), using:

1. An **MMR commitment** to all prior headers (in each tip header),
2. **Difficulty-weighted random sampling** of past headers,
3. **Merkle inclusion proofs** from samples to the tip commitment.

Related prior art: [NIPoPoW](https://eprint.iacr.org/2015/718.pdf). Production reference: [Zcash ZIP 221](https://zips.z.cash/zip-0221).

## What Grin already has (#1555 largely done)

| Piece | Location |
|-------|----------|
| Header MMR | `chain` / `PMMRHandle<BlockHeader>` |
| `BlockHeader.prev_root` | commits to prior header-MMR root |
| `HeaderEntry` leaf metadata | `hash`, `timestamp`, `total_difficulty`, secondary flags |
| Generic `MerkleProof` | `core::core::merkle_proof` |
| wtema DAA (HF4) | `consensus::next_difficulty` — needs only **2** prior blocks |

So the original “replace prev linkage with MMR root” work from #1555 is in production. What remains is the **light-client proof protocol** (and optionally a VDMMR hard fork).

## Open design question (#3479)

The paper’s **Variable Difficulty MMR** stores aggregate work/time at **parent** nodes so Merkle paths can check difficulty transitions without intermediate headers.

[John Tromp](https://github.com/mimblewimble/grin/issues/3479#issuecomment-712812386) notes that those checks appear to only enforce **feasibility bounds** on difficulty change (not full per-block DAA correctness), and that consecutive **sampled** points may already suffice for that.

With **wtema** ([RFC 0018](https://github.com/mimblewimble/grin-rfcs/blob/master/text/0018-fix-daa.md)):

- Per-block difficulty is a function of the previous block’s time delta and difficulty.
- Verifying a sampled header’s difficulty only needs its **immediate predecessor** (and genesis edge cases).
- Checking **inter-sample** cumulative difficulty vs elapsed time enforces coarse “work over time” consistency without parent aggregates.

## Decision: Path A first

| | Path A (this work) | Path B (deferred) |
|--|--------------------|-------------------|
| Consensus | **None** | Hard fork: parent hash includes aggregates |
| Proof | Samples + Merkle paths + neighbor headers for wtema | Smaller proofs if aggregates replace neighbors |
| Risk | Slightly larger proofs | Forever dual-root rules; rebuild header MMR |
| Code | Additive `core::flyclient` + later API | Revive ideas from closed [#3480](https://github.com/mimblewimble/grin/pull/3480) |

**Path A is the recommended first ship.** Path B only if security review shows ancestor aggregate checks are load-bearing under Grin’s threat model.

## Path A verification outline

Given tip header `T` (with `prev_root` = root of header MMR after applying the previous header) and proof `π`:

1. **Bootstrap:** `T` (or an ancestor) links back to a trusted genesis/checkpoint hash via `prev_hash` chain of samples or an explicit checkpoint field.
2. **Samples:** For security parameter `λ`, draw `O(λ log L)` targets uniformly in `[1, total_difficulty(T)]` (deterministic PRF seeded by tip hash).
3. **For each sample** at cumulative difficulty `d`:
   - Locate header `H` with least `total_difficulty >= d`.
   - Verify Merkle proof of `H` in the header MMR against the commitment root for the tip context.
   - Verify PoW on `H` (era-correct algorithm).
   - Verify wtema difficulty of `H` using predecessor header(s) included in the proof.
4. **Ordering:** Sampled heights/difficulties are consistent with the tip’s claimed total work.
5. **Inter-sample bounds:** Between consecutive samples (by height), the change in cumulative difficulty vs time delta is within a generous feasibility envelope (not a substitute for full DAA on every block; probabilistic as in the paper).

Exact sample-count formula and feasibility constants are parameterized (`SampleParams`) so they can be tightened after review without a format break if versioned.

## Non-goals (this phase)

- Wallet payment / output inclusion proofs (follow-on once tip is trusted).
- P2P FlyClient messages (API-first).
- VDMMR / hard fork.
- Replacing full header sync for archival nodes.

## Implementation map

| Component | Crate | Status |
|-----------|-------|--------|
| Design (this doc) | `doc/flyclient.md` | Done |
| Types, sampling, structural verify | `core/src/flyclient.rs` | Path A library |
| Prover on live chain | `chain` | Later PR |
| HTTP/JSON-RPC | `api` | Later PR |
| Light client binary | optional | Later |

## References

1. Bünz et al., *FlyClient: Super-Light Clients for Cryptocurrencies*, IEEE S&P 2020, [eprint 2019/226](https://eprint.iacr.org/2019/226).
2. Kiayias et al., *Non-Interactive Proofs of Proof-of-Work*, [eprint 2015/718](https://eprint.iacr.org/2015/718).
3. Zcash ZIP 221, *FlyClient — Consensus-Layer Changes*, [zips.z.cash/zip-0221](https://zips.z.cash/zip-0221).
4. Grin RFC 0018, *Fix DAA (wtema)*, [grin-rfcs](https://github.com/mimblewimble/grin-rfcs/blob/master/text/0018-fix-daa.md).
5. Grin issues [#1555](https://github.com/mimblewimble/grin/issues/1555), [#3479](https://github.com/mimblewimble/grin/issues/3479); closed WIP [#3480](https://github.com/mimblewimble/grin/pull/3480).
