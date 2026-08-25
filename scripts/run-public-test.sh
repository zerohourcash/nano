#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
test_root="$PWD/runtime/test-server"
api_token="$(tr -d '\n' < "$test_root/api-token")"
mesh_secret="$(tr -d '\n' < "$test_root/mesh-secret")"
exec env \
  PORT=8787 \
  DATA_DIR="$test_root/data" \
  API_TOKEN="$api_token" \
  MESH_SECRET="$mesh_secret" \
  PUBLIC_TEST_MODE=1 \
  NODE_NAME='Bit Community Public Test' \
  node src/server.js
