#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
project="ferrydock-analysis-verify-$(date +%s)-$$"
if [[ -n "${DOCKER_CONTEXT:-}" ]]; then
  endpoint="$(docker context inspect "$DOCKER_CONTEXT" --format '{{.Endpoints.docker.Host}}')"
elif [[ -n "${DOCKER_HOST:-}" ]]; then endpoint="$DOCKER_HOST"
else endpoint="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"; fi
[[ "$endpoint" == unix://* ]] || { echo 'A local Unix Docker endpoint is required' >&2; exit 1; }
export DOCKER_HOST="$endpoint"
unset DOCKER_CONTEXT
mkdir -p .verification/image-analysis
proof_dir="$(mktemp -d "$PWD/.verification/image-analysis/$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")"
printf '%s\n' "$project" > "$proof_dir/project.txt"
printf '%s\n' "$endpoint" > "$proof_dir/endpoint.txt"
cleanup() {
  local id
  while IFS= read -r id; do
    [[ -z "$id" ]] || docker image rm "$id" >>"$proof_dir/cleanup.log" 2>&1
  done < <(docker image ls -q --filter "label=dev.ferrydock.fixture.project=$project" | sort -u)
  [[ -z "$(docker image ls -q --filter "label=dev.ferrydock.fixture.project=$project")" ]]
  echo "Scoped cleanup passed: $project" >>"$proof_dir/cleanup.log"
}
trap cleanup EXIT
mkdir "$proof_dir/build"
cat > "$proof_dir/build/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.23
ARG FIXTURE_PROJECT
LABEL dev.ferrydock.fixture.project=$FIXTURE_PROJECT
COPY discard.bin /discard.bin
RUN rm /discard.bin
CMD ["true"]
DOCKERFILE
truncate -s 1048576 "$proof_dir/build/discard.bin"
docker build --build-arg "FIXTURE_PROJECT=$project" -t "$project:fixture" "$proof_dir/build" >"$proof_dir/launch.log" 2>&1
export FERRYDOCK_ANALYSIS_IMAGE FERRYDOCK_ANALYSIS_PROJECT="$project" FERRYDOCK_ANALYSIS_ENDPOINT="$endpoint" FERRYDOCK_ANALYSIS_REPORT="$proof_dir/analysis.json"
FERRYDOCK_ANALYSIS_IMAGE="$(docker image inspect --format '{{.Id}}' "$project:fixture")"
echo "Analysis evidence: $proof_dir"
if ! cargo test --locked --offline --test image_analysis -- --ignored --nocapture >"$proof_dir/test.log" 2>&1; then tail -60 "$proof_dir/test.log" >&2; exit 1; fi
cleanup
trap - EXIT
python3 - "$proof_dir" "$project" "$endpoint" <<'PY'
import json, sys
from pathlib import Path
Path(sys.argv[1], 'result.json').write_text(json.dumps({'status':'passed','fixture':sys.argv[2],'endpoint':sys.argv[3],'cleanup_success':True,'checks':['production worker Bollard export and local core','1048576 deleted payload bytes','added/deleted file changes','metadata-only history','largest final files','missing-image error']},indent=2)+'\n')
PY
