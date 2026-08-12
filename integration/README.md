# Node integration tests

Multi-node integration coverage for the Grin **node**, re-introduced after the
wallet split ([#2957](https://github.com/mimblewimble/grin/issues/2957)).

These tests do **not** depend on `grin-wallet`. Coinbase is burned via the
internal test miner (`start_test_miner(None, …)`). Wallet-coupled scenarios
(payments, dandelion with wallets, owner/foreign APIs) remain in the
[grin-wallet](https://github.com/mimblewimble/grin-wallet) repository.

## Run

From the repo root or this crate:

```bash
cd integration
cargo test --release -- --test-threads=1
```

Serial execution (`--test-threads=1`) avoids port and RocksDB races between
clusters. CI runs this crate the same way.

## Suites

| File | Coverage |
|------|----------|
| `tests/simulnet.rs` | Mining, seeding, block propagation, full/long body sync |
| `tests/p2p.rs` | Peer connect, ban, unban |
| `tests/stratum.rs` | Live stratum JSON-RPC + job broadcast |

Shared helpers live in `tests/common/`.
