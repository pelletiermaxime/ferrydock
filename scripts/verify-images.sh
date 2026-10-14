#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
project="ferrydock-images-verify-$(date +%s)-$$"
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
mkdir -p .verification/images
evidence="$(mktemp -d "$PWD/.verification/images/$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")"
export XDG_CONFIG_HOME="$evidence/settings"
cleanup() {
  local id
  while IFS= read -r id; do
    [[ -z "$id" ]] || docker rm -f "$id" >>"$evidence/cleanup.log" 2>&1
  done < <(docker ps -aq --filter "label=dev.ferrydock.fixture.project=$project")
  for tag in main tagged alias spare spare2; do
    if docker image inspect "$project:$tag" >/dev/null 2>&1; then
      docker image rm "$project:$tag" >>"$evidence/cleanup.log" 2>&1
    fi
  done
  [[ -z "$(docker ps -aq --filter "label=dev.ferrydock.fixture.project=$project")" ]]
  [[ -z "$(docker image ls -q --filter "label=dev.ferrydock.fixture.project=$project")" ]]
  echo "Scoped cleanup passed: $project"
}
trap cleanup EXIT
mkdir "$evidence/build"
cat >"$evidence/build/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.23
ARG FIXTURE_PROJECT
ARG FIXTURE_VARIANT
LABEL dev.ferrydock.fixture.project=$FIXTURE_PROJECT dev.ferrydock.fixture.variant=$FIXTURE_VARIANT
CMD ["sleep", "3600"]
DOCKERFILE
for variant in main tagged spare spare2; do
  docker build --build-arg "FIXTURE_PROJECT=$project" --build-arg "FIXTURE_VARIANT=$variant" -t "$project:$variant" "$evidence/build" >>"$evidence/launch.log" 2>&1
done
docker tag "$project:tagged" "$project:alias"
export FERRYDOCK_IMAGE_TEST_CONTAINER
FERRYDOCK_IMAGE_TEST_CONTAINER="$(docker run -d --network none --name "$project" --label "dev.ferrydock.fixture.project=$project" "$project:main")"
export FERRYDOCK_IMAGE_TEST_PROJECT="$project"
export FERRYDOCK_IMAGE_TEST_ENDPOINT="$endpoint"
export FERRYDOCK_IMAGE_TEST_ID FERRYDOCK_IMAGE_TEST_TAGGED_ID
FERRYDOCK_IMAGE_TEST_ID="$(docker image inspect --format '{{.Id}}' "$project:main")"
FERRYDOCK_IMAGE_TEST_TAGGED_ID="$(docker image inspect --format '{{.Id}}' "$project:tagged")"
export FERRYDOCK_IMAGE_TEST_SPARE_IDS
FERRYDOCK_IMAGE_TEST_SPARE_IDS="$(docker image inspect --format '{{.Id}}' "$project:spare"),$(docker image inspect --format '{{.Id}}' "$project:spare2")"
echo "Images evidence: $evidence"
if ! cargo test --locked --offline "$@" --test images_smoke -- --ignored --nocapture >"$evidence/test.log" 2>&1; then
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
  'checks':['worker discovery', 'image history commands and inspection identity/configuration', 'running and stopped references', 'guarded removal', 'multi-tag Docker conflict', 'partial bulk removal with per-image outcomes', 'successful two-image bulk removal', 'other images preserved']}, indent=2)+'\n')
PY
