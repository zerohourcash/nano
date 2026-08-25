#!/usr/bin/env bash
set -euo pipefail
export PATH="/root/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
npm test
npm audit --omit=dev
"$(dirname "$0")/smoke.sh"
