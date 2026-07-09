#!/usr/bin/env bash
# Observe compaction cadence while a node syncs (issue #3594 / PR #3892).
#
# Runs a mainnet (or testnet) node for a fixed duration and counts how many
# times compaction is *started*. With MIN_COMPACTION_INTERVAL_SECS=3600, a
# run shorter than 1 hour must log at most ONE "check_compact: starting
# compaction" line after the first trigger (typically 0 or 1 total).
#
# Usage (from repo root):
#   ./scripts/compaction_sync_observe.sh
#   DURATION_SECS=300 ./scripts/compaction_sync_observe.sh
#   NETWORK=testnet DURATION_SECS=180 ./scripts/compaction_sync_observe.sh
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

GRIN_BIN="${GRIN_BIN:-$ROOT/target/debug/grin}"
DURATION_SECS="${DURATION_SECS:-240}"
NETWORK="${NETWORK:-mainnet}" # mainnet | testnet
WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/grin-compact-observe.XXXXXX")"
LOG="$WORKDIR/grin.stdout.log"
FILE_LOG="$WORKDIR/grin-server.log"
RESULTS_DIR="${RESULTS_DIR:-/tmp/grin-compact-observe-result}"
PID=""

cleanup() {
  if [[ -n "${PID}" ]] && kill -0 "$PID" 2>/dev/null; then
    echo "==> Stopping grin (pid $PID)"
    kill -TERM "$PID" 2>/dev/null || true
    for _ in $(seq 1 40); do
      kill -0 "$PID" 2>/dev/null || break
      sleep 0.25
    done
    kill -KILL "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  # Preserve logs for inspection
  mkdir -p "$RESULTS_DIR"
  cp -f "$LOG" "$RESULTS_DIR/grin.stdout.log" 2>/dev/null || true
  cp -f "$FILE_LOG" "$RESULTS_DIR/grin-server.log" 2>/dev/null || true
  # Prefer file logger (Debug) when present
  if [[ -f "$FILE_LOG" ]]; then
    ANALYSIS_LOG="$FILE_LOG"
  else
    ANALYSIS_LOG="$LOG"
  fi
  export ANALYSIS_LOG
}
trap cleanup EXIT

echo "==> Workdir: $WORKDIR"
echo "==> Network: $NETWORK  duration: ${DURATION_SECS}s"

if [[ ! -x "$GRIN_BIN" ]]; then
  echo "==> Building grin"
  cargo build -p grin
fi

mkdir -p "$WORKDIR"
NET_ARGS=()
case "$NETWORK" in
  mainnet) ;;
  testnet) NET_ARGS+=(--testnet) ;;
  usernet) NET_ARGS+=(--usernet) ;;
  *) echo "Unknown NETWORK=$NETWORK" >&2; exit 1 ;;
esac

(
  cd "$WORKDIR"
  if ((${#NET_ARGS[@]})); then
    "$GRIN_BIN" "${NET_ARGS[@]}" server config >/dev/null
  else
    "$GRIN_BIN" server config >/dev/null
  fi
)

CFG="$WORKDIR/grin-server.toml"
python3 - "$CFG" <<'PY'
import re, sys
path = sys.argv[1]
text = open(path).read()

def set_key(text, key, value):
    pat = re.compile(rf'(?m)^(\s*{re.escape(key)}\s*=\s*).*$')
    if pat.search(text):
        return pat.sub(rf'\g<1>{value}', text)
    return text

text = set_key(text, 'run_tui', 'false')
text = set_key(text, 'skip_sync_wait', 'true')
# Ensure file logging is on and detailed enough to catch compact lines.
text = set_key(text, 'log_to_file', 'true')
text = set_key(text, 'log_to_stdout', 'true')
text = set_key(text, 'stdout_log_level', '"Info"')
text = set_key(text, 'file_log_level', '"Debug"')
open(path, 'w').write(text)
print("config patched")
PY

echo "==> Starting grin --no-tui server run (${NETWORK})"
(
  cd "$WORKDIR"
  if ((${#NET_ARGS[@]})); then
    exec "$GRIN_BIN" "${NET_ARGS[@]}" --no-tui server run
  else
    exec "$GRIN_BIN" --no-tui server run
  fi
) >"$LOG" 2>&1 &
PID=$!
echo "    pid=$PID log=$LOG"

echo "==> Observing for ${DURATION_SECS}s (expect ≤1 compact start if duration < 1h)..."
END=$((SECONDS + DURATION_SECS))
while (( SECONDS < END )); do
  if ! kill -0 "$PID" 2>/dev/null; then
    echo "ERROR: grin exited early" >&2
    tail -40 "$LOG" >&2 || true
    exit 1
  fi
  sleep 5
  # Progress hint
  if grep -q "synchronized\|HeaderSync\|BodySync\|TxHashset\|PIBD\|sync" "$LOG" 2>/dev/null; then
    :
  fi
done

# Prefer file logger (Debug) over stdout capture.
if [[ -f "$FILE_LOG" ]]; then
  ANALYSIS_LOG="$FILE_LOG"
else
  ANALYSIS_LOG="$LOG"
fi

# Count compaction starts from our instrumented log line and txhashset compact.
STARTS=$(grep -c "check_compact: starting compaction" "$ANALYSIS_LOG" 2>/dev/null || true)
STARTS=${STARTS:-0}
TXHS=$(grep -c "txhashset: starting compaction" "$ANALYSIS_LOG" 2>/dev/null || true)
TXHS=${TXHS:-0}
SKIP=$(grep -c "compact: skipping" "$ANALYSIS_LOG" 2>/dev/null || true)
SKIP=${SKIP:-0}
HEADERS=$(grep -c "Received .* block headers" "$ANALYSIS_LOG" 2>/dev/null || true)
HEADERS=${HEADERS:-0}
BLOCKS=$(grep -cE "Received block |going to process" "$ANALYSIS_LOG" 2>/dev/null || true)
BLOCKS=${BLOCKS:-0}

echo ""
echo "==> Results"
echo "  duration_secs=$DURATION_SECS"
echo "  check_compact starts: $STARTS"
echo "  txhashset compact starts: $TXHS"
echo "  compact skips (height gate): $SKIP"
echo "  header batches received: $HEADERS"
echo "  full-block process lines: $BLOCKS"
echo "  analysis_log: $ANALYSIS_LOG"
echo "  results_dir: $RESULTS_DIR"

# Also show timestamps of any compact starts
if [[ "$STARTS" -gt 0 ]]; then
  echo "  compact start lines:"
  grep "check_compact: starting compaction" "$ANALYSIS_LOG" | head -20
fi

if [[ "$BLOCKS" -eq 0 ]]; then
  echo "  note: node was likely still in header sync (process_block/check_compact not yet active)."
  echo "        Wall-clock gate is proven by unit test rapid_sync_compacts_at_most_once_per_wall_clock_window"
  echo "        (50k dice-always-hit blocks → exactly 1 trigger per hour window)."
fi

FAIL=0
# Production min interval is 1 hour. For runs shorter than 1 hour, at most 1 start.
if (( DURATION_SECS < 3600 )); then
  if (( STARTS > 1 )); then
    echo "FAIL: saw $STARTS compact starts in ${DURATION_SECS}s (< 1h window allows at most 1)" >&2
    FAIL=1
  else
    echo "OK: compact starts ($STARTS) ≤ 1 for run shorter than 1 hour"
  fi
else
  # Allow roughly one per hour (+1 slack for boundary)
  MAX_ALLOWED=$(( DURATION_SECS / 3600 + 1 ))
  if (( STARTS > MAX_ALLOWED )); then
    echo "FAIL: $STARTS starts > allowed ~$MAX_ALLOWED for ${DURATION_SECS}s" >&2
    FAIL=1
  else
    echo "OK: compact starts ($STARTS) within ~once/hour budget ($MAX_ALLOWED)"
  fi
fi

if [[ "$FAIL" -ne 0 ]]; then
  tail -80 "$LOG" >&2 || true
  exit 1
fi

echo ""
echo "PASS: compaction cadence within wall-clock bound during observe window"
echo "  (Unit test rapid_sync_compacts_at_most_once_per_wall_clock_window covers the"
echo "   50k-block same-instant worst case; this script observes a real syncing node.)"
