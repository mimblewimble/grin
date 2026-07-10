#!/usr/bin/env bash
# Simulate a weak VPS (slow CPU / tight RAM) and hard-kill mid PMMR compaction.
#
# Testnet is usually the best real-world repro (often heavier than mainnet even
# on a desktop): high spend churn + first compact after PIBD rewrites almost
# the entire hash/data files. Use --testnet-like for that profile.
#
# Modes:
#   ./run.sh                  # native host: kill-test at journal phase
#   ./run.sh --testnet-like   # high prune ratio + larger leaf count + slow-only timing
#   ./run.sh --weak           # docker: 0.5 CPU, 512MB, platform linux/amd64 (QEMU on arm)
#   ./run.sh --qemu-user      # linux host: run binary under qemu-x86_64 (TCG, very slow)
#   ./run.sh --all-phases     # kill-test at hash_tmp, data_tmp, and journal
#   ./run.sh --slow-only      # prepare + compact only (time first vs second compact)
#
# Env overrides:
#   LEAVES=100000      number of leaves
#   PRUNE_PCT=90       percent of leaves pruned before compact (testnet-like: 90-95)
#   PHASE=journal      kill phase: hash_tmp | data_tmp | journal
#   WORK_DIR=...       data directory (default under target/tmp)
#
# Exit 0 on success.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

MODE="native"
ALL_PHASES=0
SLOW_ONLY=0
TESTNET_LIKE=0
PHASE="${PHASE:-journal}"
WORK_DIR="${WORK_DIR:-$ROOT/target/tmp/qemu-compact-stress}"

while [[ $# -gt 0 ]]; do
	case "$1" in
		--weak) MODE="weak"; shift ;;
		--qemu-user) MODE="qemu-user"; shift ;;
		--all-phases) ALL_PHASES=1; shift ;;
		--slow-only) SLOW_ONLY=1; shift ;;
		--testnet-like) TESTNET_LIKE=1; SLOW_ONLY=1; shift ;;
		--phase) PHASE="$2"; shift 2 ;;
		--leaves) LEAVES="$2"; shift 2 ;;
		--prune-pct) PRUNE_PCT="$2"; shift 2 ;;
		-h|--help)
			sed -n '2,28p' "$0"
			exit 0
			;;
		*)
			echo "unknown arg: $1" >&2
			exit 2
			;;
	esac
done

if [[ "$TESTNET_LIKE" -eq 1 ]]; then
	# First compact after a dense prune set (post-PIBD / high-churn testnet).
	LEAVES="${LEAVES:-100000}"
	PRUNE_PCT="${PRUNE_PCT:-92}"
elif [[ -z "${LEAVES:-}" ]]; then
	if [[ "$MODE" == "weak" ]]; then
		LEAVES=8000
	else
		LEAVES=20000
	fi
fi
PRUNE_PCT="${PRUNE_PCT:-50}"

echo "==> building compact_crash_stress (release)"
echo "    leaves=${LEAVES} prune_pct=${PRUNE_PCT} mode=${MODE} phase=${PHASE}"
cargo build -p grin_store --example compact_crash_stress --release
BIN="$ROOT/target/release/examples/compact_crash_stress"
test -x "$BIN"

run_bin() {
	if [[ "$MODE" == "qemu-user" ]]; then
		local qemubin=""
		if command -v qemu-x86_64 >/dev/null 2>&1; then
			qemubin="qemu-x86_64"
		elif command -v qemu-x86_64-static >/dev/null 2>&1; then
			qemubin="qemu-x86_64-static"
		else
			echo "qemu-x86_64 not found (install qemu-user)" >&2
			exit 1
		fi
		# TCG user-mode: intentionally slow, like a tiny VPS CPU.
		"$qemubin" -cpu qemu64 "$BIN" "$@"
	else
		"$BIN" "$@"
	fi
}

run_kill_phase() {
	local phase="$1"
	local dir="${WORK_DIR}/${phase}"
	rm -rf "$dir"
	mkdir -p "$dir"
	echo ""
	echo "========== kill-test phase=${phase} leaves=${LEAVES} prune_pct=${PRUNE_PCT} mode=${MODE} =========="
	run_bin kill-test "$dir" "$LEAVES" "$phase" "$PRUNE_PCT"
}

run_slow_only() {
	local dir="${WORK_DIR}/slow"
	rm -rf "$dir"
	mkdir -p "$dir"
	echo ""
	echo "========== slow compact leaves=${LEAVES} prune_pct=${PRUNE_PCT} mode=${MODE} =========="
	echo "    (first compact ≈ testnet post-PIBD; second ≈ incremental mainnet delta)"
	run_bin prepare "$dir" "$LEAVES" "$PRUNE_PCT"
	run_bin compact "$dir"
	run_bin verify "$dir"
}

run_native_or_qemu() {
	if [[ "$SLOW_ONLY" -eq 1 ]]; then
		run_slow_only
		return
	fi
	if [[ "$ALL_PHASES" -eq 1 ]]; then
		for p in hash_tmp data_tmp journal; do
			run_kill_phase "$p"
		done
	else
		run_kill_phase "$PHASE"
	fi
}

run_weak_docker() {
	if ! command -v docker >/dev/null 2>&1; then
		echo "docker not found; cannot run --weak mode" >&2
		exit 1
	fi

	# linux/amd64 under Docker Desktop on Apple Silicon uses QEMU TCG.
	local image="${COMPACT_STRESS_IMAGE:-rust:1.83-bookworm}"
	local platform="${COMPACT_STRESS_PLATFORM:-linux/amd64}"

	echo "==> weak VPS via docker (${platform}, 0.5 CPU, 512MB) image=${image}"
	echo "    (on arm hosts Docker uses QEMU to emulate amd64 — expect multi-minute runs)"

	docker run --rm \
		--platform "$platform" \
		--cpus="0.5" \
		--memory="512m" \
		--memory-swap="512m" \
		-v "$ROOT:/src:rw" \
		-w /src \
		-e CARGO_TARGET_DIR=/src/target/qemu-compact-docker \
		"$image" \
		bash -lc "
			set -euo pipefail
			apt-get update -qq && apt-get install -y -qq build-essential pkg-config libclang-dev >/dev/null
			cargo build -p grin_store --example compact_crash_stress --release
			BIN=/src/target/qemu-compact-docker/release/examples/compact_crash_stress
			WORK=/src/target/tmp/qemu-compact-stress-docker
			rm -rf \"\$WORK\"
			mkdir -p \"\$WORK\"
			if [[ '${SLOW_ONLY}' -eq 1 ]]; then
				\"\$BIN\" prepare \"\$WORK/slow\" ${LEAVES} ${PRUNE_PCT}
				\"\$BIN\" compact \"\$WORK/slow\"
				\"\$BIN\" verify \"\$WORK/slow\"
			elif [[ '${ALL_PHASES}' -eq 1 ]]; then
				for p in hash_tmp data_tmp journal; do
					echo \"========== docker kill-test phase=\$p ==========\"
					\"\$BIN\" kill-test \"\$WORK/\$p\" ${LEAVES} \"\$p\" ${PRUNE_PCT}
				done
			else
				\"\$BIN\" kill-test \"\$WORK/${PHASE}\" ${LEAVES} ${PHASE} ${PRUNE_PCT}
			fi
		"
}

case "$MODE" in
	native|qemu-user) run_native_or_qemu ;;
	weak) run_weak_docker ;;
	*) echo "bad mode $MODE" >&2; exit 2 ;;
esac

echo ""
echo "OK: compact crash stress finished (mode=${MODE} leaves=${LEAVES} prune_pct=${PRUNE_PCT})"
