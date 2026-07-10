#!/usr/bin/env bash
# Simulate a weak VPS (slow CPU / tight RAM) and hard-kill mid PMMR compaction.
#
# Modes:
#   ./run.sh                  # native host: kill-test at journal phase
#   ./run.sh --weak           # docker: 1 CPU, 512MB, platform linux/amd64 (QEMU on arm hosts)
#   ./run.sh --qemu-user      # linux host: run binary under qemu-x86_64 (TCG, very slow)
#   ./run.sh --all-phases     # kill-test at hash_tmp, data_tmp, and journal
#   ./run.sh --slow-only      # prepare + compact only (time slow compact, no kill)
#
# Env overrides:
#   LEAVES=50000       number of leaves (default 20000 native, 8000 weak)
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
PHASE="${PHASE:-journal}"
WORK_DIR="${WORK_DIR:-$ROOT/target/tmp/qemu-compact-stress}"

while [[ $# -gt 0 ]]; do
	case "$1" in
		--weak) MODE="weak"; shift ;;
		--qemu-user) MODE="qemu-user"; shift ;;
		--all-phases) ALL_PHASES=1; shift ;;
		--slow-only) SLOW_ONLY=1; shift ;;
		--phase) PHASE="$2"; shift 2 ;;
		--leaves) LEAVES="$2"; shift 2 ;;
		-h|--help)
			sed -n '2,20p' "$0"
			exit 0
			;;
		*)
			echo "unknown arg: $1" >&2
			exit 2
			;;
	esac
done

if [[ -z "${LEAVES:-}" ]]; then
	if [[ "$MODE" == "weak" ]]; then
		LEAVES=8000
	else
		LEAVES=20000
	fi
fi

echo "==> building compact_crash_stress (release)"
cargo build -p grin_store --example compact_crash_stress --release
BIN="$ROOT/target/release/examples/compact_crash_stress"
test -x "$BIN"

run_bin() {
	# shellcheck disable=SC2086
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
		# TCG softmmu user-mode: intentionally slow, like a tiny VPS CPU.
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
	echo "========== kill-test phase=${phase} leaves=${LEAVES} mode=${MODE} =========="
	run_bin kill-test "$dir" "$LEAVES" "$phase"
}

run_slow_only() {
	local dir="${WORK_DIR}/slow"
	rm -rf "$dir"
	mkdir -p "$dir"
	echo ""
	echo "========== slow compact leaves=${LEAVES} mode=${MODE} =========="
	run_bin prepare "$dir" "$LEAVES"
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
	# Combined with --cpus=0.5 --memory=512m this approximates a weak VPS.
	local image="${COMPACT_STRESS_IMAGE:-rust:1.83-bookworm}"
	local platform="${COMPACT_STRESS_PLATFORM:-linux/amd64}"

	echo "==> weak VPS via docker (${platform}, 0.5 CPU, 512MB) image=${image}"
	echo "    (on arm hosts Docker uses QEMU to emulate amd64 — expect multi-minute runs)"

	# Build inside the container so the binary matches the platform.
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
				\"\$BIN\" prepare \"\$WORK/slow\" ${LEAVES}
				\"\$BIN\" compact \"\$WORK/slow\"
				\"\$BIN\" verify \"\$WORK/slow\"
			elif [[ '${ALL_PHASES}' -eq 1 ]]; then
				for p in hash_tmp data_tmp journal; do
					echo \"========== docker kill-test phase=\$p ==========\"
					\"\$BIN\" kill-test \"\$WORK/\$p\" ${LEAVES} \"\$p\"
				done
			else
				\"\$BIN\" kill-test \"\$WORK/${PHASE}\" ${LEAVES} ${PHASE}
			fi
		"
}

case "$MODE" in
	native|qemu-user) run_native_or_qemu ;;
	weak) run_weak_docker ;;
	*) echo "bad mode $MODE" >&2; exit 2 ;;
esac

echo ""
echo "OK: compact crash stress finished (mode=${MODE})"
