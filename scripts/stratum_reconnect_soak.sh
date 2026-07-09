#!/usr/bin/env bash
# Full-node soak: run grin with stratum enabled + reconnecting "miner" clients.
# Verifies worker connections and process FDs stay bounded (issue #3867 / PR #3889).
#
# Usage (from repo root):
#   ./scripts/stratum_reconnect_soak.sh
#
# Env overrides:
#   GRIN_BIN       path to grin binary (default: target/debug/grin)
#   CYCLES         connect/login/drop cycles (default: 120)
#   CONCURRENT     concurrent held connections (default: 32)
#   FD_GROWTH_MAX  max allowed FD growth over soak (default: 100)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

GRIN_BIN="${GRIN_BIN:-$ROOT/target/debug/grin}"
CYCLES="${CYCLES:-120}"
CONCURRENT="${CONCURRENT:-32}"
FD_GROWTH_MAX="${FD_GROWTH_MAX:-100}"
STRATUM_HOST="127.0.0.1"
# Ephemeral high port avoids collisions with leftover usernet nodes.
STRATUM_PORT="${STRATUM_PORT:-$(( 24000 + (RANDOM % 1000) ))}"
API_PORT="${API_PORT:-$(( 25000 + (RANDOM % 1000) ))}"
P2P_PORT="${P2P_PORT:-$(( 26000 + (RANDOM % 1000) ))}"
WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/grin-stratum-soak.XXXXXX")"
LOG="$WORKDIR/grin.log"
PID=""

cleanup() {
  if [[ -n "${PID}" ]] && kill -0 "$PID" 2>/dev/null; then
    echo "==> Stopping grin (pid $PID)"
    kill -TERM "$PID" 2>/dev/null || true
    for _ in $(seq 1 30); do
      kill -0 "$PID" 2>/dev/null || break
      sleep 0.2
    done
    kill -KILL "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

count_fds() {
  local pid="$1"
  if command -v lsof >/dev/null 2>&1; then
    # Count open files for the process (includes sockets).
    lsof -nP -p "$pid" 2>/dev/null | tail -n +2 | wc -l | tr -d ' '
  elif [[ -d "/proc/$pid/fd" ]]; then
    ls -1 "/proc/$pid/fd" 2>/dev/null | wc -l | tr -d ' '
  else
    echo "0"
  fi
}

count_tcp_established_stratum() {
  local pid="$1"
  if command -v lsof >/dev/null 2>&1; then
    # ESTABLISHED only (exclude LISTEN). Match local or remote stratum port.
    lsof -nP -p "$pid" -iTCP 2>/dev/null \
      | awk -v p=":${STRATUM_PORT}" '
          NR>1 && $0 ~ /ESTABLISHED/ && index($0, p) { c++ }
          END { print c+0 }
        '
  else
    echo "0"
  fi
}

wait_port() {
  local host="$1" port="$2" tries="${3:-60}"
  for i in $(seq 1 "$tries"); do
    if (echo >/dev/tcp/"$host"/"$port") >/dev/null 2>&1; then
      return 0
    fi
    # bash /dev/tcp may be unavailable; fall back to python
    if python3 - "$host" "$port" <<'PY' 2>/dev/null; then
import socket, sys
s = socket.socket()
s.settimeout(0.3)
try:
    s.connect((sys.argv[1], int(sys.argv[2])))
    sys.exit(0)
except Exception:
    sys.exit(1)
finally:
    s.close()
PY
      return 0
    fi
    sleep 0.5
  done
  return 1
}

echo "==> Workdir: $WORKDIR"
echo "==> Building grin if needed"
if [[ ! -x "$GRIN_BIN" ]]; then
  cargo build -p grin
fi

echo "==> Generating usernet config with stratum enabled"
mkdir -p "$WORKDIR"
(
  cd "$WORKDIR"
  "$GRIN_BIN" --usernet server config >/dev/null
)

CFG="$WORKDIR/grin-server.toml"
if [[ ! -f "$CFG" ]]; then
  echo "ERROR: expected $CFG after 'grin server config'" >&2
  exit 1
fi

# Kill any leftover listener on our chosen ports (best-effort).
if command -v lsof >/dev/null 2>&1; then
  for p in "$STRATUM_PORT" "$API_PORT" "$P2P_PORT"; do
    lsof -nP -iTCP:"$p" -sTCP:LISTEN -t 2>/dev/null | xargs kill -9 2>/dev/null || true
  done
fi

# Enable stratum, disable TUI, skip sync wait, burn rewards (no wallet needed).
export STRATUM_PORT API_PORT P2P_PORT
python3 - "$CFG" <<'PY'
import re, sys
path = sys.argv[1]
text = open(path).read()

def set_key(text, key, value):
    # Match key = ... at line start (optional spaces)
    pat = re.compile(rf'(?m)^(\s*{re.escape(key)}\s*=\s*).*$')
    if pat.search(text):
        return pat.sub(rf'\g<1>{value}', text)
    return text

import os
stratum = os.environ["STRATUM_PORT"]
api = os.environ["API_PORT"]
p2p = os.environ["P2P_PORT"]
text = set_key(text, 'run_tui', 'false')
text = set_key(text, 'skip_sync_wait', 'true')
text = set_key(text, 'enable_stratum_server', 'true')
text = set_key(text, 'burn_reward', 'true')
text = set_key(text, 'stratum_server_addr', f'"127.0.0.1:{stratum}"')
text = set_key(text, 'api_http_addr', f'"127.0.0.1:{api}"')
# p2p listen port field is just `port` under [server.p2p_config]
# Prefer the commented-uncommented style: replace first bare port = under p2p section via line walk.
lines = text.splitlines(True)
out = []
in_p2p = False
for line in lines:
    if line.strip().startswith('[server.p2p_config]'):
        in_p2p = True
        out.append(line)
        continue
    if in_p2p and line.strip().startswith('['):
        in_p2p = False
    if in_p2p and re.match(r'^\s*port\s*=', line):
        out.append(f'port = {p2p}\n')
        continue
    out.append(line)
text = ''.join(out)
# Keep usernet local-only (must remain a quoted TOML string)
text = set_key(text, 'seeding_type', '"None"')
open(path, 'w').write(text)
print(f"config patched stratum=127.0.0.1:{stratum} api={api} p2p={p2p}")
PY

echo "==> Starting grin --usernet --no-tui server run"
# Run grin as the background job itself (not a wrapper subshell) so FD accounting
# via lsof -p targets the real server process.
(
  cd "$WORKDIR"
  exec "$GRIN_BIN" --usernet --no-tui server run
) >"$LOG" 2>&1 &
PID=$!
echo "    pid=$PID log=$LOG"

echo "==> Waiting for stratum port ${STRATUM_HOST}:${STRATUM_PORT}"
if ! wait_port "$STRATUM_HOST" "$STRATUM_PORT" 90; then
  echo "ERROR: stratum port never opened" >&2
  echo "---- grin log (tail) ----" >&2
  tail -80 "$LOG" >&2 || true
  exit 1
fi
echo "    stratum is accepting connections"

# Allow a moment for server threads to settle.
sleep 1
FD_BEFORE=$(count_fds "$PID")
TCP_BEFORE=$(count_tcp_established_stratum "$PID")
echo "==> Baseline FDs=$FD_BEFORE established-stratum-TCP=$TCP_BEFORE"

echo "==> Reconnect storm: $CYCLES connect/login/drop cycles"
python3 - "$STRATUM_HOST" "$STRATUM_PORT" "$CYCLES" "$CONCURRENT" <<'PY'
import socket, sys, time

host, port = sys.argv[1], int(sys.argv[2])
cycles, concurrent = int(sys.argv[3]), int(sys.argv[4])
login = (
    b'{"id":1,"jsonrpc":"2.0","method":"login",'
    b'"params":{"login":"soak","pass":"x","agent":"soak-script"}}\n'
)
keepalive = b'{"id":2,"jsonrpc":"2.0","method":"keepalive","params":null}\n'

def connect_login():
    s = socket.create_connection((host, port), timeout=3)
    s.settimeout(1.0)
    s.sendall(login)
    try:
        s.recv(1024)
    except Exception:
        pass
    return s

# Phase 1: reconnect storm
for i in range(cycles):
    s = connect_login()
    s.close()
    if (i + 1) % 40 == 0:
        print(f"  cycle {i+1}/{cycles}", flush=True)

# Phase 2: hold concurrent connections, then drop
print(f"  holding {concurrent} concurrent connections", flush=True)
held = []
for _ in range(concurrent):
    held.append(connect_login())
time.sleep(1.0)
# Send keepalive on each so idle path is exercised a bit
for s in held:
    try:
        s.sendall(keepalive)
        s.recv(1024)
    except Exception:
        pass
time.sleep(0.5)
for s in held:
    s.close()
print("  concurrent connections closed", flush=True)
# Allow server tasks to observe EOF and free workers
time.sleep(1.5)
print("miner simulation done", flush=True)
PY

FD_AFTER=$(count_fds "$PID")
TCP_AFTER=$(count_tcp_established_stratum "$PID")
FD_DELTA=$((FD_AFTER - FD_BEFORE))
echo "==> After soak: FDs=$FD_AFTER (delta=$FD_DELTA) established-stratum-TCP=$TCP_AFTER"

# Give a little more time for cleanup
sleep 1
FD_FINAL=$(count_fds "$PID")
TCP_FINAL=$(count_tcp_established_stratum "$PID")
echo "==> Final:     FDs=$FD_FINAL established-stratum-TCP=$TCP_FINAL"

echo "==> Checking bounds"
FAIL=0
if [[ "$FD_DELTA" -gt "$FD_GROWTH_MAX" ]]; then
  echo "FAIL: FD growth $FD_DELTA exceeds limit $FD_GROWTH_MAX" >&2
  FAIL=1
else
  echo "OK: FD growth $FD_DELTA <= $FD_GROWTH_MAX"
fi

# After drop, should not still hold client ESTABLISHED sockets to stratum.
if [[ "$TCP_FINAL" -gt 4 ]]; then
  echo "FAIL: still have $TCP_FINAL ESTABLISHED TCP to stratum after clients closed" >&2
  FAIL=1
else
  echo "OK: established stratum TCP after close = $TCP_FINAL (expected ~0)"
fi

if ! kill -0 "$PID" 2>/dev/null; then
  echo "FAIL: grin process died during soak" >&2
  tail -40 "$LOG" >&2 || true
  FAIL=1
else
  echo "OK: grin still running"
fi

# Ignore bind panics from concurrent leftover nodes on shared default ports;
# with unique ports a bind panic is a real failure.
if grep -qi "Too many open files" "$LOG"; then
  echo "FAIL: EMFILE in log" >&2
  grep -i "Too many open files" "$LOG" | tail -20 >&2 || true
  FAIL=1
elif grep -qi "panicked" "$LOG"; then
  echo "FAIL: panic in log" >&2
  grep -i "panicked" "$LOG" | tail -20 >&2 || true
  FAIL=1
else
  echo "OK: no panic/EMFILE in log"
fi

if [[ "$FAIL" -ne 0 ]]; then
  echo "---- grin log (tail) ----" >&2
  tail -60 "$LOG" >&2 || true
  exit 1
fi

echo ""
echo "PASS: full-node stratum reconnect soak bounded"
echo "  workdir=$WORKDIR"
echo "  cycles=$CYCLES concurrent=$CONCURRENT fd_before=$FD_BEFORE fd_after=$FD_AFTER"
