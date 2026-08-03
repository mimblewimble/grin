#!/usr/bin/env bash
# Simulate a weak VPS (slow CPU / tight RAM) and hard-kill mid PMMR compaction.
#
# Testnet is usually the best real-world repro (often heavier than mainnet even
# on a desktop): high spend churn + first compact after PIBD rewrites almost
# the entire hash/data files. Use --testnet-like for that profile.
#
# Modes:
#   ./run.sh                  # native: auto kill-test at journal (you do NOT kill)
#   ./run.sh --testnet-like   # timing only (no kill): large leaves + ~92% prune
#   ./run.sh --weak           # docker cgroup limits (0.5 CPU, 512MB), host arch
#   ./run.sh --qemu-user      # linux: run under qemu-x86_64 (TCG)
#   ./run.sh --all-phases     # kill-test at hash_tmp, data_tmp, journal
#   ./run.sh --slow-only      # prepare + compact only (no kill)
#
# You never need to Ctrl-C / kill by hand for the harness — kill-test sends
# SIGKILL to a child automatically. Only a real grin --testnet node would use
# manual kill -9 while logs show compaction.
#
# Env overrides:
#   LEAVES=100000      number of leaves
#   PRUNE_PCT=90       percent pruned (testnet-like: 90-95)
#   PHASE=journal      kill phase
#   WORK_DIR=...
#   COMPACT_STRESS_IMAGE=rust:1.83-bookworm
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
			sed -n '2,32p' "$0"
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
	# Smaller default under --weak so Docker builds finish in reasonable time.
	if [[ "$MODE" == "weak" ]]; then
		LEAVES="${LEAVES:-20000}"
	else
		LEAVES="${LEAVES:-100000}"
	fi
	PRUNE_PCT="${PRUNE_PCT:-92}"
elif [[ -z "${LEAVES:-}" ]]; then
	if [[ "$MODE" == "weak" ]]; then
		LEAVES=8000
	else
		LEAVES=20000
	fi
fi
PRUNE_PCT="${PRUNE_PCT:-50}"

echo "==> compact crash stress"
echo "    leaves=${LEAVES} prune_pct=${PRUNE_PCT} mode=${MODE} phase=${PHASE}"
if [[ "$SLOW_ONLY" -eq 1 ]]; then
	echo "    mode=timing only — do NOT kill; wait for prepare/compact progress on stderr"
else
	echo "    mode=kill-test — harness auto-SIGKILLs a child at phase=${PHASE} (no manual kill)"
fi

echo "==> building compact_crash_stress (release) on host"
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
	echo "========== kill-test phase=${phase} leaves=${LEAVES} prune_pct=${PRUNE_PCT} =========="
	echo "    (parent waits for phase, then SIGKILLs child — no action needed from you)"
	run_bin kill-test "$dir" "$LEAVES" "$phase" "$PRUNE_PCT"
}

run_slow_only() {
	local dir="${WORK_DIR}/slow"
	rm -rf "$dir"
	mkdir -p "$dir"
	echo ""
	echo "========== slow compact leaves=${LEAVES} prune_pct=${PRUNE_PCT} =========="
	echo "    first compact ≈ testnet post-PIBD; second ≈ already-compacted delta"
	echo "    No kill: leave this running until it prints 'OK: compact crash stress finished'"
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

	# Use the host's native platform (linux/arm64 on Apple Silicon, linux/amd64
	# on typical CI). Forcing linux/amd64 on arm runs the whole toolchain under
	# QEMU TCG and often breaks PATH / takes hours. Cgroup limits still model a
	# weak VPS; for extra CPU drag use --qemu-user on an x86_64 Linux host.
	local image="${COMPACT_STRESS_IMAGE:-rust:1.83-bookworm}"
	local platform
	platform="$(docker version -f '{{.Server.Os}}/{{.Server.Arch}}' 2>/dev/null || echo linux/amd64)"

	echo "==> weak VPS via docker"
	echo "    platform=${platform} (native)  cpus=0.5  memory=512m  image=${image}"
	echo "    building inside container (first run downloads the image + deps)"

	# Explicit cargo PATH: login shells under some images do not load rustup env.
	docker run --rm \
		--platform "$platform" \
		--cpus="0.5" \
		--memory="512m" \
		--memory-swap="512m" \
		-v "$ROOT:/src:rw" \
		-w /src \
		-e CARGO_TARGET_DIR=/src/target/qemu-compact-docker \
		-e PATH="/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
		"$image" \
		bash -c "
			set -euo pipefail
			export PATH=\"/usr/local/cargo/bin:\$PATH\"
			if [[ -f /usr/local/cargo/env ]]; then
				# shellcheck disable=SC1091
				source /usr/local/cargo/env
			fi
			if ! command -v cargo >/dev/null 2>&1; then
				echo \"cargo not found in image PATH=\$PATH\" >&2
				ls -la /usr/local/cargo/bin 2>/dev/null || true
				exit 1
			fi
			echo \"==> docker: cargo=\$(command -v cargo) rustc=\$(rustc --version)\"
			# Keep apt light — only if clang headers missing for bindgen crates.
			if ! dpkg -s libclang-dev >/dev/null 2>&1; then
				apt-get update -qq
				DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential pkg-config libclang-dev
			fi
			echo '==> docker: cargo build (release example) — may take a while under cgroup limits'
			cargo build -p grin_store --example compact_crash_stress --release
			BIN=/src/target/qemu-compact-docker/release/examples/compact_crash_stress
			WORK=/src/target/tmp/qemu-compact-stress-docker
			rm -rf \"\$WORK\"
			mkdir -p \"\$WORK\"
			if [[ '${SLOW_ONLY}' -eq 1 ]]; then
				echo '==> docker: prepare+compact (no kill)'
				\"\$BIN\" prepare \"\$WORK/slow\" ${LEAVES} ${PRUNE_PCT}
				\"\$BIN\" compact \"\$WORK/slow\"
				\"\$BIN\" verify \"\$WORK/slow\"
			elif [[ '${ALL_PHASES}' -eq 1 ]]; then
				for p in hash_tmp data_tmp journal; do
					echo \"========== docker kill-test phase=\$p ==========\"
					\"\$BIN\" kill-test \"\$WORK/\$p\" ${LEAVES} \"\$p\" ${PRUNE_PCT}
				done
			else
				echo \"==> docker: kill-test phase=${PHASE} (auto SIGKILL)\"
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
