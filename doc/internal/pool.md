# Transaction Pool

This document describes the design and behavior of Grin's transaction pool as implemented today in the `pool` crate (`pool/src/`).

For Dandelion stem/fluff propagation details, see [dandelion.md](../dandelion/dandelion.md).

## Purpose

The transaction pool keeps a set of unconfirmed transactions that:

1. Can be selected by the mining service when building a new block.
2. Can be relayed to peers (after any stem embargo ends).
3. Moderate broadcast behavior so only transactions valid against current chain state (plus the pool itself) are accepted.

The pool is required for mining and for normal transaction relay. It is implemented as two related layers plus a short re-org cache:

| Layer | Name in code | Visibility | Role |
|-------|--------------|------------|------|
| **txpool** | `TransactionPool.txpool` | Public | Fluffed transactions ready for relay and mining |
| **stempool** | `TransactionPool.stempool` | Private | Stem-phase Dandelion transactions under embargo |
| **reorg cache** | `TransactionPool.reorg_cache` | Internal | Recent fluffed txs kept briefly to repopulate after a reorg |

Both `txpool` and `stempool` are instances of the same `Pool` type (`pool/src/pool.rs`), parameterized by a `BlockChain` adapter for UTXO and lock-height checks.

## Design: one big transaction

Grin does **not** store the mempool as a DAG of transactions.

Earlier design notes (and older versions of this document) described a pair of directed acyclic graphs for connected and orphan transactions. That model was abandoned.

The current model treats each pool as a flat list of `PoolEntry` values (transaction + source + timestamp), stored in insertion order:

```text
Pool.entries: Vec<PoolEntry>
```

Validation does not walk a graph. Instead, **all transactions already in a pool are aggregated into a single transaction** (Mimblewimble cut-through / aggregation). Accepting a new transaction means:

> Aggregate `existing_pool_txs + new_tx` and check that this single aggregate is valid against the current chain state (at a given header).

Conceptually the validation stack is nested aggregation over chain state:

```text
[ new_tx + [ stempool + [ txpool + [ chain_state ] ] ] ]
```

More precisely:

- **Adding to txpool:** validate  
  `aggregate(txpool_txs ∪ {new_tx})` against `chain_state`.
- **Adding to stempool:** validate  
  `aggregate(stempool_txs ∪ {new_tx} ∪ txpool_aggregate)` against `chain_state`,  
  so stem txs cannot conflict with either the chain or the public txpool.
- After the txpool changes, the stempool is **reconciled**: each stem entry is re-checked against the updated txpool aggregate and dropped if no longer valid.

This is why `Pool::all_transactions_aggregate` and `transaction::aggregate` are central to the implementation: the pool is always considered as “one big tx” (or empty) on top of the UTXO set.

### Why aggregation instead of a DAG

- Mimblewimble already supports transaction aggregation and cut-through at the protocol level.
- A single aggregate validation reuses the same rules as block validation (kernel sums, UTXO membership, coinbase maturity, NRD rules, etc.).
- Dependency ordering is not required for storage; it is reconstructed only when selecting txs for a block (see [Mining selection](#mining-selection)).

There is no separate orphan pool. A transaction whose inputs are not in the UTXO set and not created by other pool transactions simply fails validation and is rejected.

## Layers in detail

### txpool

- Public mempool used for network fluff and for building blocks.
- `TransactionPool::total_size` and capacity checks that affect the node’s reported pool size refer to the **txpool only** (stempool is under embargo and not advertised).
- On successful fluff acceptance, the entry is also pushed into the reorg cache and the pool adapter’s `tx_accepted` hook runs (P2P broadcast, etc.).

### stempool

- Private holding area for Dandelion **stem** transactions.
- Stem txs are validated with the current **txpool aggregate** as `extra_tx`, so they cannot double-spend against fluffed txs.
- Contents are not used for compact-block reconstruction or mining (`retrieve_transactions` and `prepare_mineable_transactions` use **txpool only**).
- The Dandelion monitor (`servers/src/grin/dandelion_monitor.rs`) periodically fluffs stem entries when embargo or epoch timers expire by moving them into the txpool path.

If a stem transaction is already in the stempool and is seen again as stem, the pool **fluffs** it (adds to txpool) instead of treating it as a duplicate stem. Duplicates already in the txpool return `PoolError::DuplicateTx`.

### reorg cache

- Stores recent successfully fluffed `PoolEntry` values (same size bound as `max_pool_size`).
- On a reorg that increases total work, `reconcile_reorg_cache` attempts to re-add those entries to the txpool against the new tip.
- Entries older than the configured retention window (`reorg_cache_period`, default 30 minutes) are truncated.

## Adding a transaction

High-level flow in `TransactionPool::add_to_pool`:

1. **Duplicate / stem-to-fluff checks** against stempool and txpool (by matching kernels).
2. **Deaggregation** (fluff path only): if the new tx has multiple kernels, try to strip kernels that already exist in the txpool (`transaction::deaggregate`) so the remainder can be accepted cleanly.
3. **Kernel variant checks** (e.g. NRD only when enabled and header version allows).
4. **Policy checks** (`is_acceptable`): pool capacity, minimum fee vs weight (`accept_fee_base`).
5. **Transaction validation** (`tx.validate` as a standalone tx).
6. **Lock height** against current chain.
7. **Locate spends** in pool outputs vs UTXO (`locate_spends` + cut-through); enforce coinbase maturity on UTXO spends.
8. **Input format conversion** to v2 “features and commit” inputs when needed for relay.
9. **Insert** into stempool or txpool via aggregate validation (`Pool::add_to_pool`).
10. On fluff: update reorg cache, notify adapter; optionally **evict** a low-priority tx if the pool was over capacity.

Failed stem adapter acceptance falls back to fluff (add to txpool).

## Mining selection

`prepare_mineable_transactions` (txpool only):

1. Sort and group pool txs with **bucket** logic (`bucket_transactions`):
   - Prefer keeping dependency order and maximizing cut-through within a bucket.
   - Prefer higher aggregate fee rate; avoid merging if it would lower fee rate.
   - Txs with multiple in-pool parents are skipped for this block (picked up later).
2. Re-validate the ordered list incrementally as aggregates against chain state under the miner’s `mineable_max_weight`.
3. Return the list of individually mineable transactions for block building.

Eviction under capacity pressure uses the same bucket ordering and removes a last (low fee-rate, non-dependent) transaction.

## Reconciliation with the chain

When a new block is accepted on the main chain:

1. Remove txs from txpool/stempool that are included or conflicted via `reconcile_block` (kernel / input based).
2. Re-apply remaining txpool entries against the new header (`reconcile`).
3. Re-apply stempool entries against the new header **and** the updated txpool aggregate.

After a reorg, the reorg cache is used to try restoring recently fluffed transactions.

## Configuration (pool)

See `PoolConfig` and `DandelionConfig` in `pool/src/types.rs`. Important fields:

| Setting | Role |
|---------|------|
| `accept_fee_base` | Minimum fee scale for acceptance |
| `max_pool_size` | Max txpool entries (also reorg cache hard cap) |
| `max_stempool_size` | Max stempool entries |
| `mineable_max_weight` | Weight budget when selecting txs for a block |
| `reorg_cache_period` | How long (minutes) to retain reorg-cache entries |

Dandelion timings (epoch, embargo, aggregation, stem probability) live in `DandelionConfig` and are shared with the p2p / monitor logic.

## Adversarial conditions

Primary concerns remain denial-of-service and resource exhaustion:

- Capacity limits on txpool and stempool.
- Minimum fee relative to transaction weight.
- Aggregate validation cost is paid on add; invalid aggregates are rejected before insertion.
- Stempool privacy: stem contents are not exposed via pool query APIs used for compact blocks.

The critical invariant for miners is that `prepare_mineable_transactions` returns a set that still validates against current chain state under the configured weight limit, even if the pool is under load.

## Code map

| Component | Location |
|-----------|----------|
| `TransactionPool` (txpool + stempool + reorg cache) | `pool/src/transaction_pool.rs` |
| `Pool` (entries, aggregate validate, buckets, mining prep) | `pool/src/pool.rs` |
| Config, `PoolEntry`, errors, adapters | `pool/src/types.rs` |
| Dandelion stem timers / fluff | `servers/src/grin/dandelion_monitor.rs` |
| Block building from pool | `servers/src/mining/mine_block.rs` |
| Reconcile on new block / reorg | `servers/src/common/adapters.rs` |

## Historical note

Documentation historically described the mempool as a pair of DAGs (connected graph + orphans) with explicit parent edges per input. The implementation no longer uses that structure; it uses ordered vectors of transactions and **aggregate “one big tx” validation**, with separate **txpool** and **stempool** instances for Dandelion. This document matches the latter design.
