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
fixture_directory="$evidence/fixture"
# Keep a real, owned linked checkout in the evidence for source discovery proof.
git init --quiet "$evidence/repository"
git -C "$evidence/repository" -c user.name=Fixture -c user.email=fixture@example.invalid -c commit.gpgsign=false -c core.hooksPath=/dev/null commit --quiet --allow-empty -m fixture
git -C "$evidence/repository" worktree add --quiet --detach "$fixture_directory"
cp verify/compose-project.yaml verify/compose-project.override.yaml verify/compose-project.cleanup.yaml verify/compose-project.env verify/compose-project.Dockerfile "$fixture_directory/"
printf 'before-rebuild\n' > "$fixture_directory/rebuild-marker"
printf '\nFERRY_COMPOSE_BUILD_IMAGE=ferrydock-compose-verify:%s\nFERRY_COMPOSE_BUILD_PROJECT=%s\n' "$project" "$project" >> "$fixture_directory/compose-project.env"
fixture=(docker --host "$endpoint" compose --project-name "$project" --project-directory "$fixture_directory" --file "$fixture_directory/compose-project.yaml" --file "$fixture_directory/compose-project.override.yaml" --env-file "$fixture_directory/compose-project.env" --profile extra)
cleanup() {
  "${fixture[@]}" down --volumes --remove-orphans >"$evidence/cleanup.log" 2>&1
  local remaining
  remaining="$(docker --host "$endpoint" ps -aq --filter "label=com.docker.compose.project=$project")"
  [[ -z "$remaining" ]]
  remaining="$(docker --host "$endpoint" network ls -q --filter "label=com.docker.compose.project=$project")"
  [[ -z "$remaining" ]]
  # Include superseded images after rebuilding, scoped by this run's build label.
  local image
  while IFS= read -r image; do
    [[ -z "$image" ]] || docker --host "$endpoint" image rm "$image" >>"$evidence/cleanup.log" 2>&1
  done < <(docker --host "$endpoint" image ls -q --filter "label=dev.ferrydock.fixture.project=$project" | sort -u)
  remaining="$(docker --host "$endpoint" image ls -q --filter "label=dev.ferrydock.fixture.project=$project")"
  [[ -z "$remaining" ]]
  # Cleanup-specific test resources, including owned external preservation probes.
  local volume network
  while IFS= read -r volume; do
    [[ -z "$volume" ]] || docker --host "$endpoint" volume rm "$volume" >>"$evidence/cleanup.log" 2>&1
  done < <(docker --host "$endpoint" volume ls -q --filter "label=dev.ferrydock.fixture.project=$project")
  while IFS= read -r network; do
    [[ -z "$network" ]] || docker --host "$endpoint" network rm "$network" >>"$evidence/cleanup.log" 2>&1
  done < <(docker --host "$endpoint" network ls -q --filter "label=dev.ferrydock.fixture.project=$project")
  [[ -z "$(docker --host "$endpoint" volume ls -q --filter "label=dev.ferrydock.fixture.project=$project")" ]]
  [[ -z "$(docker --host "$endpoint" network ls -q --filter "label=dev.ferrydock.fixture.project=$project")" ]]
  # The test records this exact anonymous mount from its owned fixture container.
  if [[ -f "$fixture_directory/cleanup-anonymous-volume" ]]; then
    local anonymous
    anonymous="$(cat "$fixture_directory/cleanup-anonymous-volume")"
    [[ "$anonymous" =~ ^[a-f0-9]{64}$ ]]
    if [[ -n "$(docker --host "$endpoint" volume ls -q --filter "name=^$anonymous$")" ]]; then
      docker --host "$endpoint" volume rm "$anonymous" >>"$evidence/cleanup.log" 2>&1
    fi
  fi
  echo "Scoped cleanup passed: $project"
}
trap cleanup EXIT
"${fixture[@]}" up -d --build >"$evidence/launch.log" 2>&1
export FERRYDOCK_COMPOSE_TEST_PROJECT="$project"
export FERRYDOCK_COMPOSE_TEST_ENDPOINT="$endpoint"
export FERRYDOCK_COMPOSE_TEST_DIRECTORY="$fixture_directory"
export FERRYDOCK_COMPOSE_TEST_EVIDENCE="$evidence"
# Deliberately hostile ambient values must not alter the saved invocation.
export COMPOSE_FILE=/nonexistent/ferrydock-ambient.yaml
export COMPOSE_PROFILES=nonexistent-profile
export COMPOSE_PROJECT_NAME=unrelated-project
echo "Compose evidence: $evidence"
cargo test --locked --offline "$@" --test compose_smoke -- --ignored --nocapture 2>&1 | tee "$evidence/test.log"
