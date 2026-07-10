# QEMU / weak-VPS compaction crash stress

Reproduces the failure mode from low-end VPS hosts (issue #3872):

1. **Slow compaction** — large prune set rewritten under a throttled CPU
2. **Hard kill mid-compact** — `SIGKILL` while staging files or after the journal
3. **Recovery on reopen** — root must still match; no leftover journal

## Pieces

| Path | Role |
|------|------|
| `store/examples/compact_crash_stress.rs` | Harness: prepare / compact / verify / kill-test |
| `etc/qemu-compact-stress/run.sh` | Runner: native, qemu-user, or Docker+QEMU weak VPS |
| `GRIN_COMPACT_*` env hooks in `store/src/pmmr.rs` | Pause at a compact phase so the parent can kill reliably |

Test hooks are **no-ops** unless the env vars are set (production paths unchanged).

## Quick start (native)

```bash
# Kill at journal commit (the critical hard-kill window), 20k leaves
./etc/qemu-compact-stress/run.sh

# All phases: pre-journal staging + journal
./etc/qemu-compact-stress/run.sh --all-phases

# Time a slow compact only (no kill)
./etc/qemu-compact-stress/run.sh --slow-only LEAVES=50000

# Or call the example directly
cargo run -p grin_store --example compact_crash_stress --release -- \
  kill-test ./target/tmp/my-kill 10000 journal
```

## Weak VPS via Docker + QEMU

Uses `linux/amd64` under Docker. On Apple Silicon this is **QEMU TCG** emulation,
plus cgroup limits (`0.5` CPU, `512MB`) — close to a cheap VPS.

```bash
# Expect multi-minute runs on arm hosts
./etc/qemu-compact-stress/run.sh --weak

# Slow compact only under the same constraints
./etc/qemu-compact-stress/run.sh --weak --slow-only

# Override leaf count / phase
LEAVES=5000 PHASE=journal ./etc/qemu-compact-stress/run.sh --weak
```

Requirements: Docker with buildx/platform emulation enabled.

## qemu-user (Linux CI / x86_64 host)

```bash
# Install qemu-user, then:
./etc/qemu-compact-stress/run.sh --qemu-user --slow-only LEAVES=10000
```

TCG user-mode intentionally slows the binary to stress the rewrite path.

## Kill phases

| Phase | When | Expected recovery |
|-------|------|-------------------|
| `hash_tmp` | After hash staging file written | Discard orphans; pre-compact live files |
| `data_tmp` | After data staging file written | Same |
| `journal` | After journal fsynced, before renames | Roll renames forward; post-compact state |

## Manual env for debugging

```bash
export GRIN_COMPACT_READY_FILE=/tmp/compact_ready
export GRIN_COMPACT_PAUSE_PHASE=journal
export GRIN_COMPACT_CRASH_PAUSE_MS=60000
# run compact in one terminal, kill -9 when ready file says "journal"
```
