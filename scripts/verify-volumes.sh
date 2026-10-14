#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
project="ferrydock-volumes-verify-$(date +%s)-$$"
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
mkdir -p .verification/volumes
evidence="$(mktemp -d "$PWD/.verification/volumes/$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")"
export XDG_CONFIG_HOME="$evidence/settings"
cleanup() {
  local id name containers volumes remaining
  containers="$(docker ps -aq --filter "label=dev.ferrydock.fixture.project=$project")" || return 1
  while IFS= read -r id; do
    [[ -z "$id" ]] || docker rm -f "$id" >>"$evidence/cleanup.log" 2>&1 || return 1
  done <<<"$containers"
  volumes="$(docker volume ls -q --filter "label=dev.ferrydock.fixture.project=$project")" || return 1
  while IFS= read -r name; do
    [[ -z "$name" ]] || docker volume rm "$name" >>"$evidence/cleanup.log" 2>&1 || return 1
  done <<<"$volumes"
  remaining="$(docker ps -aq --filter "label=dev.ferrydock.fixture.project=$project")" || return 1
  [[ -z "$remaining" ]] || return 1
  remaining="$(docker volume ls -q --filter "label=dev.ferrydock.fixture.project=$project")" || return 1
  [[ -z "$remaining" ]] || return 1
  echo "Scoped cleanup passed: $project"
}
trap cleanup EXIT
for variant in used unused preserved bulk-a bulk-b; do
  docker volume create --label "dev.ferrydock.fixture.project=$project" --label "com.docker.compose.project=$project" "$project-$variant" >>"$evidence/launch.log" 2>&1
done
export FERRYDOCK_VOLUME_TEST_CONTAINER
FERRYDOCK_VOLUME_TEST_CONTAINER="$(docker run -d --network none --name "$project" --label "dev.ferrydock.fixture.project=$project" --mount "type=volume,src=$project-used,dst=/data" alpine:3.23 sh -c 'dd if=/dev/zero of=/data/size-fixture.bin bs=1048576 count=3 2>/dev/null; exec sleep 3600')"
export FERRYDOCK_VOLUME_TEST_PROJECT="$project"
export FERRYDOCK_VOLUME_TEST_ENDPOINT="$endpoint"
echo "Volumes evidence: $evidence"
if ! cargo test --locked --offline "$@" --test volumes_smoke -- --ignored --nocapture >"$evidence/test.log" 2>&1; then
  cat "$evidence/test.log" >&2
  exit 1
fi
cat "$evidence/test.log"
cleanup
trap - EXIT
python3 - "$evidence" "$project" "$endpoint" <<'PY'
import json, sys
from pathlib import Path
Path(sys.argv[1], 'result.json').write_text(json.dumps({'status':'passed', 'fixture':sys.argv[2], 'endpoint':sys.argv[3], 'cleanup_success':True,
  'checks':['worker discovery and configuration', 'explicit volume-only disk usage with known 3 MiB payload', 'normal discovery never gathers sizes', 'running and stopped container references', 'referenced removal blocked', 'partial bulk removal with protected/missing targets', 'successful two-volume bulk removal', 'unused volume removed', 'inventory reconciliation', 'other volumes preserved', 'released reference removal', 'missing-volume error']}, indent=2)+'\n')
PY
