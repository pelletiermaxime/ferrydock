#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
project="ferrydock-compose-verify-$(date +%s)-$$"
if [[ -n "${DOCKER_CONTEXT:-}" ]]; then
  endpoint="$(docker context inspect "$DOCKER_CONTEXT" --format '{{.Endpoints.docker.Host}}')"
elif [[ -n "${DOCKER_HOST:-}" ]]; then
  endpoint="$DOCKER_HOST"
else
  endpoint="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
fi
[[ "$endpoint" == unix://* ]] || { echo "This check requires a local Unix Docker endpoint" >&2; exit 1; }
export DOCKER_HOST="$endpoint"
unset DOCKER_CONTEXT
mkdir -p .verification/compose
evidence="$(mktemp -d "$PWD/.verification/compose/$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")"
export XDG_CONFIG_HOME="$evidence/settings"
fixture=(docker --host "$endpoint" compose --project-name "$project" --project-directory "$PWD/verify" --file "$PWD/verify/compose-project.yaml" --file "$PWD/verify/compose-project.override.yaml" --env-file "$PWD/verify/compose-project.env" --profile extra)
cleanup() {
  "${fixture[@]}" down --volumes --remove-orphans >"$evidence/cleanup.log" 2>&1
  local remaining
  remaining="$(docker --host "$endpoint" ps -aq --filter "label=com.docker.compose.project=$project")"
  [[ -z "$remaining" ]]
  remaining="$(docker --host "$endpoint" network ls -q --filter "label=com.docker.compose.project=$project")"
  [[ -z "$remaining" ]]
  echo "Scoped cleanup passed: $project"
}
trap cleanup EXIT
"${fixture[@]}" up -d >"$evidence/launch.log" 2>&1
export FERRYDOCK_COMPOSE_TEST_PROJECT="$project"
export FERRYDOCK_COMPOSE_TEST_ENDPOINT="$endpoint"
export FERRYDOCK_COMPOSE_TEST_DIRECTORY="$PWD/verify"
export FERRYDOCK_COMPOSE_TEST_EVIDENCE="$evidence"
# Deliberately hostile ambient values must not alter the saved invocation.
export COMPOSE_FILE=/nonexistent/ferrydock-ambient.yaml
export COMPOSE_PROFILES=nonexistent-profile
export COMPOSE_PROJECT_NAME=unrelated-project
echo "Compose evidence: $evidence"
cargo test --locked --offline "$@" --test compose_smoke -- --ignored --nocapture 2>&1 | tee "$evidence/test.log"
