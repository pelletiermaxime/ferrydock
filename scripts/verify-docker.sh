#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
project="ferrydock-verify-$(date +%s)-$$"
fixture=(docker compose --project-name "$project" --file verify/compose.yaml)
cleanup() { "${fixture[@]}" down --volumes --remove-orphans; }
trap cleanup EXIT
"${fixture[@]}" up -d
export FERRYDOCK_TEST_CONTAINER
FERRYDOCK_TEST_CONTAINER="$("${fixture[@]}" ps -q probe)"
cargo test --offline --locked "$@" --test docker_smoke -- --ignored --nocapture
