#!/usr/bin/env bash
set -euo pipefail
export PATH="/root/.cargo/bin:$PATH"
smoke_dir="$(mktemp -d)"
smoke_port="${BIT_SMOKE_PORT:-38787}"
smoke_token="smoke-test-token-0123456789abcdef"
cleanup(){ if [[ -n "${smoke_pid:-}" ]]; then kill "$smoke_pid" 2>/dev/null || true; wait "$smoke_pid" 2>/dev/null || true; fi; rm -rf "$smoke_dir"; }
trap cleanup EXIT
cargo build -p bit-node
BIT_ADMIN_TOKEN="$smoke_token" BIT_DATA_DIR="$smoke_dir" BIT_LISTEN="127.0.0.1:$smoke_port" target/debug/bit-node >"$smoke_dir/node.log" 2>&1 &
smoke_pid=$!
for _ in {1..50}; do curl -fsS "http://127.0.0.1:$smoke_port/health" >/dev/null && break; sleep 0.1; done
curl -fsS -X POST "http://127.0.0.1:$smoke_port/v1/admin/assets" -H "Authorization: Bearer $smoke_token" -H 'Content-Type: application/json' -d '{"name":"Smoke drill","serial":"SM-1","location":"A","value_minor":100}' >/dev/null
curl -fsS "http://127.0.0.1:$smoke_port/v1/state" | grep -q 'Smoke drill'
kill "$smoke_pid";wait "$smoke_pid" || true;unset smoke_pid
BIT_ADMIN_TOKEN="$smoke_token" BIT_DATA_DIR="$smoke_dir" BIT_LISTEN="127.0.0.1:$smoke_port" target/debug/bit-node >"$smoke_dir/restart.log" 2>&1 &
smoke_pid=$!
for _ in {1..50}; do curl -fsS "http://127.0.0.1:$smoke_port/v1/state" 2>/dev/null | grep -q 'Smoke drill' && break; sleep 0.1; done
curl -fsS "http://127.0.0.1:$smoke_port/v1/state" | grep -q 'Smoke drill'
