# QEMU / weak-VPS / testnet-like compaction stress

Reproduces slow compaction and hard-kill mid-compact (issue #3872).

## Why testnet is worse than mainnet

Cut-through horizon is the same on both chains (one week of blocks), but in
practice **testnet is usually the easier and heavier repro**, even on a local PC:

| Factor | Testnet | Long-running mainnet |
|--------|---------|----------------------|
| Spent / UTXO churn | High (faucets, testing, spam) | Lower relative churn |
| Typical compact delta | Often large | Incremental small rewrites |
| After PIBD / fresh node | First compact rewrites almost all hash+data against a huge prune set | Same worst case if you never compacted, but less often |
| Desktoplocal timing | Easy to notice multi-second / multi-minute runs | Often mild if node has been compacting for months |

So: **reproduce on testnet first**, then use this harness to isolate the PMMR
rewrite + kill/recovery path without running a full node.

## Real testnet repro (full node)

```bash
# Sync testnet (PIBD or existing chain_data), then force/wait for compact.
# In another shell, when logs show compact starting:
kill -9 $(pgrep -f 'grin.*testnet')   # hard kill mid-compact
# Restart; with the journal fix the node should recover without a bad root.
grin --testnet
```

Watch for `txhashset: starting compaction` / `check_compact` and for a long
`Loading database` / high iowait during rewrite (same class of load as #3872).

## Harness pieces

| Path | Role |
|------|------|
| `store/examples/compact_crash_stress.rs` | prepare / compact / verify / kill-test |
| `etc/qemu-compact-stress/run.sh` | native · testnet-like · qemu-user · Docker weak VPS |
| `GRIN_COMPACT_*` env hooks in `store/src/pmmr.rs` | pause at a phase so parent can SIGKILL |

Hooks are **no-ops** unless env vars are set.

## Quick start

```bash
# Testnet-like: ~100k leaves, ~92% pruned, time first vs second compact
./etc/qemu-compact-stress/run.sh --testnet-like

# Kill at journal with a dense prune set (closer to post-PIBD)
LEAVES=50000 PRUNE_PCT=92 ./etc/qemu-compact-stress/run.sh

# All kill windows
./etc/qemu-compact-stress/run.sh --all-phases

# Weak VPS: Docker linux/amd64 + 0.5 CPU + 512MB (QEMU TCG on Apple Silicon)
./etc/qemu-compact-stress/run.sh --weak --testnet-like

# Direct example
cargo run -p grin_store --example compact_crash_stress --release -- \
  prepare ./target/tmp/tn 100000 92
cargo run -p grin_store --example compact_crash_stress --release -- \
  compact ./target/tmp/tn
```

`compact` prints **first** and **second** pass times:

- **first** ≈ testnet / first compact after a dense prune set  
- **second** ≈ incremental mainnet-style compact (no new prunes)  

Expect first ≫ second; that gap is exactly why testnet “feels heavier”.

## Kill phases

| Phase | When | Expected recovery |
|-------|------|-------------------|
| `hash_tmp` | After hash staging | Discard orphans; keep pre-compact live files |
| `data_tmp` | After data staging | Same |
| `journal` | After journal fsync, before renames | Roll renames forward; post-compact state |

## Manual env

```bash
export GRIN_COMPACT_READY_FILE=/tmp/compact_ready
export GRIN_COMPACT_PAUSE_PHASE=journal
export GRIN_COMPACT_CRASH_PAUSE_MS=60000
# run compact in one terminal; kill -9 when ready file says "journal"
```
