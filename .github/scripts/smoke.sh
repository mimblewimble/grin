#!/bin/sh

set -eu

binary=$1
smoke_dir=$(mktemp -d "${TMPDIR:-/tmp}/grin-smoke.XXXXXX")
node_pid=

# Stop the node and remove its temporary data.
cleanup() {
	if [ -n "$node_pid" ] && kill -0 "$node_pid" 2>/dev/null; then
		kill "$node_pid" 2>/dev/null || true
		wait "$node_pid" 2>/dev/null || true
	fi
	rm -rf "$smoke_dir"
}

trap cleanup EXIT
trap 'exit 1' HUP INT TERM

cd "$smoke_dir"
"$binary" --usernet server config
api_addr=$(grep '^api_http_addr = ' grin-server.toml | cut -d '"' -f 2)
if [ -z "$api_addr" ]; then
	cat grin-server.toml
	exit 1
fi
"$binary" --usernet --no-tui server run >node.log 2>&1 &
node_pid=$!

# Wait up to one minute for the owner API.
attempt=0
while [ "$attempt" -lt 60 ]; do
	if ! kill -0 "$node_pid" 2>/dev/null; then
		cat node.log
		exit 1
	fi
	if [ -f .api_secret ]; then
		secret=$(head -n 1 .api_secret)
		response=$(curl --noproxy '*' -fsS \
			--user "grin:$secret" \
			--header 'Content-Type: application/json' \
			--data '{"jsonrpc":"2.0","method":"get_status","params":null,"id":1}' \
			"http://$api_addr/v2/owner" 2>/dev/null) || true
		if printf '%s' "$response" | tr -d '[:space:]' | grep -q '"chain":"user"'; then
			printf '%s\n' "$response"
			exit 0
		fi
	fi
	attempt=$((attempt + 1))
	sleep 1
done

cat node.log
exit 1
