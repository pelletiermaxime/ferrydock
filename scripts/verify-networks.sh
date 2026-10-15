#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
project="ferrydock-networks-verify-$(date +%s)-$$"
if [[ -n "${DOCKER_CONTEXT:-}" ]]; then
  endpoint="$(docker context inspect "$DOCKER_CONTEXT" --format '{{.Endpoints.docker.Host}}')"
elif [[ -n "${DOCKER_HOST:-}" ]]; then
  endpoint="$DOCKER_HOST"
else
  endpoint="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
fi
[[ "$endpoint" == unix://* ]] || { echo "A local Unix Docker endpoint is required" >&2; exit 1; }
export DOCKER_HOST="$endpoint"
unset DOCKER_CONTEXT
mkdir -p .verification/networks
evidence="$(mktemp -d "$PWD/.verification/networks/$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")"
export XDG_CONFIG_HOME="$evidence/settings"
cleanup() {
  local id remaining
  remaining="$(docker ps -aq --filter "label=dev.ferrydock.fixture.project=$project")" || return 1
  while IFS= read -r id; do
    [[ -z "$id" ]] || docker rm -f "$id" >>"$evidence/cleanup.log" 2>&1 || return 1
  done <<<"$remaining"
  remaining="$(docker network ls -q --filter "label=dev.ferrydock.fixture.project=$project")" || return 1
  while IFS= read -r id; do
    [[ -z "$id" ]] || docker network rm "$id" >>"$evidence/cleanup.log" 2>&1 || return 1
  done <<<"$remaining"
  remaining="$(docker ps -aq --filter "label=dev.ferrydock.fixture.project=$project")" || return 1
  [[ -z "$remaining" ]] || return 1
  remaining="$(docker network ls -q --filter "label=dev.ferrydock.fixture.project=$project")" || return 1
  [[ -z "$remaining" ]] || return 1
  echo "Scoped cleanup passed: $project"
}
trap cleanup EXIT
for variant in attached empty; do
  docker network create --label "dev.ferrydock.fixture.project=$project" --label "com.docker.compose.project=$project" "$project-$variant" >>"$evidence/launch.log" 2>&1
done
export FERRYDOCK_NETWORK_TEST_API FERRYDOCK_NETWORK_TEST_DB
FERRYDOCK_NETWORK_TEST_API="$(docker run -d --name "$project-api" --network "$project-attached" --network-alias backend --label "dev.ferrydock.fixture.project=$project" alpine:3.23 sleep 3600)"
FERRYDOCK_NETWORK_TEST_DB="$(docker run -d --name "$project-db" --network "$project-attached" --network-alias database --label "dev.ferrydock.fixture.project=$project" alpine:3.23 sleep 3600)"
docker stop "$FERRYDOCK_NETWORK_TEST_DB" >>"$evidence/launch.log" 2>&1
export FERRYDOCK_NETWORK_TEST_PROJECT="$project" FERRYDOCK_NETWORK_TEST_ENDPOINT="$endpoint"
echo "Networks evidence: $evidence"
if ! cargo test --locked --offline "$@" --test networks_smoke -- --ignored --nocapture >"$evidence/test.log" 2>&1; then
  tail -60 "$evidence/test.log" >&2
  exit 1
fi
tail -12 "$evidence/test.log"
cleanup
trap - EXIT
python3 - "$evidence" "$project" "$endpoint" <<'PY'
import json, sys
from pathlib import Path
Path(sys.argv[1], 'result.json').write_text(json.dumps({'status':'passed', 'fixture':sys.argv[2], 'endpoint':sys.argv[3], 'cleanup_success':True,
 'checks':['production worker inventory', 'running and stopped configured members', 'IPv4 CIDR and aliases from Docker inspect', 'stopped aliases without active IP', 'subnet and gateway', 'empty network', 'disconnect membership reconciliation']}, indent=2)+'\n')
PY
