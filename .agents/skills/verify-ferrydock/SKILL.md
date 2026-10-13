---
name: verify-ferrydock
description: Verify FerryDock with fast Linux headless GPU screenshots and GPUI input tests, real Compose worker checks, or a native window and disposable Docker fixture. Use for development checks, containers/search, logs, terminal, inspect, container controls, and Compose project associations/actions. Preserve screenshots, actions, results and scoped cleanup evidence.
---

# Verify FerryDock

Work from the repository root. Read [features/README.md](features/README.md), then the feature being exercised. Use headless GPU verification for the everyday UI loop, Docker integration checks for daemon behavior, and native-window checks for desktop integration. The headless flow uses the real FerryDock view, WGPU renderer and GPUI input dispatch, with a fixture at the production Docker worker channel boundary. Browser automation cannot drive this app.

## Launch

The default development check needs a Linux GPU/driver usable by WGPU, Python 3.11+, and cached Cargo dependencies. It needs neither a display server nor Docker:

```sh
python3 scripts/verify-headless.py
```

The helper removes `DISPLAY` and `WAYLAND_DISPLAY` only from its child environment, enables `headless-tests`, and builds/runs the current source with the locked dependency graph. FerryDock uses GPUI Kit `0.7.3` with `gpui-fast` enabled and matching Fast engine/platform/WGPU version `0.1.7`; fetch once with `cargo fetch --locked` if the cache is missing. The result records both engine package and version. It supplies the Linux headless renderer explicitly; GPU initialization failure fails the test, with no no-op renderer fallback. Each run owns a virtual GPUI window/context, fixture channels and a unique evidence directory. It never starts the Docker worker or creates a compositor window. Cold compilation is distinct from warm execution.

For native-window verification, build the shipping configuration before testing changed Rust code:

```sh
cargo build --release --locked
```

The Linux desktop needs an accessible X11 display, Docker Engine with a local Unix socket, Docker Compose, `xdotool`, ImageMagick's `magick`, Tesseract with English data, and Python 3 with Pillow. GPUI's system build dependencies are listed in the project README. This workstation was observed on display `:0`; use the actual display on another workstation. The application normally selects native Wayland when available. This X11-specific harness removes `WAYLAND_DISPLAY` only from its child environment so its window ownership and input checks still work; it does not prove native Wayland integration.

```sh
python3 .agents/skills/verify-ferrydock/scripts/native.py doctor --display :0
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature terminal --display :0
```

`run` performs launch → doctor → drive → evidence → cleanup in one owned session. It creates a fresh `ferrydock-verify-<random>` Compose project from [fixture.compose.yaml](scripts/fixture.compose.yaml), then launches its own application process. It never adopts an existing FerryDock window. The first fixture launch may pull `alpine:3.23`. The image cache remains afterward.

The Docker daemon is shared with the user's projects. The fixture has a unique project/name, ownership labels, and its own network. It has no published ports, host bind mounts, or persistent volumes. The application may read other containers during initial discovery; the helper filters to its own project and proves selection before performing any product mutation. It resolves the active Docker context once and pins that Unix endpoint only in its child environments.

## Doctor

For the short-lived headless flow, require process exit 0, `result.json` with `status: passed` and `owned_process_exited: true`, plus all listed PNGs. Runtime initialization validates the renderer; assertions require real painted glyphs, nonblank images, scale-correct dimensions and a visible image change after typed search. A Cargo success without a completed flow report is insufficient. Inspect `test.log` first when anything fails; never mark a missing GPU or failed assertion as skipped/passed.

For native runs, require `ready: true` from `native.py doctor`. It checks tools, display access, daemon/Compose access, source freshness, and the native-input lock. It reports the binary SHA-256 and endpoint. A successful build alone does not prove a working native session.

After launch, the helper checks `/proc/<pid>/exe`, a visible window matching **both** the owned PID and exact title, fixture labels, and focus. The full fixture name must appear in both the list and selected detail header before a feature runs. If focus moves elsewhere, the helper refuses input. Do not manually drive the desktop during an automated run.

If doctor fails, fix that precondition first; do not switch silently to a user's existing application, a different daemon, mock state, or a debug build. Desktop/Docker access may require sandbox escalation. Keep the commands and ownership scope unchanged when requesting access.

## Drive

Run `python3 scripts/verify-headless.py` for the mapped development flow. It seeds container/log events through the worker channel, then uses keyboard events for search/selection and Enter, and pointer events at rendered debug-selector bounds for log-follow toggles and Stop. It asserts the outgoing Stop/terminal commands and terminal input bytes, and paints ANSI fixture output through the actual terminal view. No private app setters or direct action-handler calls drive the flow. This proves UI requests and rendering, not real container lifecycle or shell execution; use `scripts/verify-docker.sh` or the native fixture for those boundaries.

The PNGs are visual evidence and the test checks meaningful image changes; there is no golden-image comparison yet. Clipboard/compositor behavior, other terminal entry points and the full feature inventory remain outside this flow. It can run while the user works on their desktop because it sends no desktop input. A verifier subagent can run this command and return `result.json`, relevant failures from `test.log`, and screenshot paths without editing the app or inventing coverage.

The Kit control flow additionally traverses to a real Button with Tab and activates
it through Enter/Space, checks loading/disabled action protection, clicks both
TabBars by their native scoped integer IDs, and drags Resizable using measured
pane bounds. Inspect tests use fixture-only JSON, count syntax-colored pixels
inside actual editor text bounds, reject editing, and drive the built-in search.
Notifications are in-app; their close control is revealed by hover. The harness
commits hover hitboxes before a click and settles both painted animation time and
the simulated lifecycle clock before asserting dismissal or persistence.

For FerryDock verifier delegation, the user's preferred configuration is `gpt-6-luna` with `high` reasoning effort. Use a fresh bounded verifier agent with these settings; an existing agent created with `low` effort keeps its original setting. Have it run the existing checks and inspect screenshots without editing app code. Report concrete visible defects and distinguish normal scroll viewport clipping from content overlapping another pane. Do not automatically broaden the task into implementation or unrelated verification.

The same headless flow also clicks a project heading, edits the Compose association form through keyboard events, and asserts ordered files, environment files and profiles. It checks disabled unassociated/conflicting controls, outgoing Start/Stop/Restart requests, and live/failure output using fixture events. Inspect all three Compose screenshots when this feature changes. No Compose subprocess is launched by headless tests.

Project headings default to the combined Logs tab. The headless flow proves the project stream request without association, source-prefixed records, service-filter rendered rows, follow/pause, copy feedback, recreated-member rendering and return-to-container routing. Inspect the all-services, filtered and recreated-member frames. Copy text formatting has a unit check; the headless action does not prove the operating system clipboard. See [features/project-logs.md](features/project-logs.md).

Busy-log verification fills the retained 1,500-line history and feeds 10,000 additional records in batches. Require `log-performance.json` to show fewer than 80 rendered rows, the retained history bound, newest-line visibility, `follow_reached_tail: true` and `controls_respond_after_flood: true`. Inspect the project-logs-busy and project-logs-batched-busy frames. These measurements exercise rendering and fixture-channel ingestion, not native CPU or real Docker throughput. The container list and both log lists are virtualized; horizontal scrolling intentionally keeps long log records on one line.

ANSI log verification uses the existing headless flow: split colour escapes in container output, green project payload with neutral metadata/other service, and coloured Compose output replacing a partial `Starting` line with `Started` through CR/erase-in-line. Inspect the containers-and-logs, project-logs-all, project-logs-filtered and compose-operation-live frames. Assertions check actual colour pixels within the corresponding rendered log-row bounds, excluding similarly coloured controls; they also require readable copy text without escape bytes. Unit checks cover resets, RGB/indexed styles, per-container decoder state, split UTF-8, progress editing, timestamp scope and bounds. This is a log-record renderer, not full-screen terminal emulation; native clipboard and every visual SGR attribute remain outside the headless proof.

For development-build log performance, intentionally build with `cargo build --locked`, then run `python3 scripts/verify-log-performance.py --display :0`. Use `--binary target/release/ferrydock` after a release build to measure shipping performance. This separate check owns an X11/XWayland window and ten uniquely labelled disposable containers producing roughly 500 lines/s. It takes desktop focus, uses the native-input lock, validates PID/executable/window/fixture ownership, checks the selected project header and live output, measures CPU/RSS, types a no-match search under load, and cleans up its process and fixture. Require `result.json` status passed, `controls_respond_under_load`, `owned_process_exited` and `cleanup_success`. Capture/OCR timings are reported separately; CPU is an observation on this workstation, not a portable threshold. This does not verify native Wayland or clipboard integration. Do not run alongside another native-input session.

Add `--ansi` to that helper to emit real green UTF-8 fixture records. Require `ansi_fixture: true` in the result and inspect the retained native log screenshot for coloured payloads/neutral metadata with no literal escape codes. This exercises the production Docker stream through the owned native window under load; do not claim native filter/pause/copy controls were driven.

For real Compose process behavior, run `bash scripts/verify-compose.sh`. This creates a unique `ferrydock-compose-verify-<timestamp>-<pid>` two-service project with base/override files, an environment file and a profile. It pins the current local Unix endpoint, isolates `XDG_CONFIG_HOME` under its evidence directory, drives production worker commands, checks daemon effects and streamed CLI output, and removes only that project on exit. It deliberately supplies conflicting ambient Compose variables to prove that saved settings win. It does not drive native GPUI input. Read [features/compose.md](features/compose.md) for the combined proof boundary.

For native desktop integration, choose one mapped flow; each invocation owns a new session and cleans it up:

The embedded-terminal flow is the exercised baseline. Mouse-dependent flows
currently stop safely at hover calibration on the observed multi-monitor
desktop; inspect is source-backed and has not been run. Read [PROOF.md](PROOF.md)
before selecting broader coverage. Do not treat these additional commands as
passing checks until the native display path has been demonstrated.

```sh
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature containers --display :0
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature logs --display :0
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature terminal --display :0
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature controls --display :0
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature inspect --display :0
```

The feature map distinguishes exercised flows from source-backed entry points. A script branch's existence does not prove it passed on a desktop. If a sub-feature is not covered, report it as unverified and add a real front-door step before making a coverage claim.

Keyboard input is targeted at the verified window. Mouse flows locate visible labels with OCR and require observable hover calibration before the first click; unsupported display scaling is a failed precondition. No fixed screen coordinates, synthetic application state setters, or direct Docker lifecycle calls may substitute for a product action.

## Evidence

The headless helper prints its unique evidence directory under `.verification/headless/`. It retains `result.json` (engine package/version, lock hash, command, exit status, timing and cleanup), `headless.json` (checks, dimensions, per-frame render/encoding timings), `log-performance.json`, `actions.json`, `test.log`, and twenty-three PNGs. The virtual display currently uses 2x scaling, producing 2360x1640 pixels for a 1180x820 logical view; dimensions are checked against the reported scale. Inspect the initial list, empty-search, Kit Input editing, log scrollbars, resized dock, Inspect JSON/search, notifications, terminal, project Logs and Compose operation frames when reviewing visual changes. Timing includes builds at the outer level; the flow's timing excludes compilation.

Real Compose checks retain `result.json`, `test.log`, `launch.log`, `cleanup.log`, and isolated association settings under `.verification/compose/<run>/`. Require the test to pass and the script's scoped cleanup confirmation. Results prove worker/CLI/daemon behavior separately from headless rendering; do not claim native Compose clicks were exercised.

The real Compose check also follows both fixture services through the production Docker API, corroborates stdout/stderr source identity, restarts the project, and force-recreates only the fixture's extra service to verify new-container log attachment. The result includes initial, restarted and recreated log samples. Recreation is a fixture setup action here, not a claimed FerryDock Recreate control.

Native runs print their unique evidence directory under `.verification/ferrydock/`. Both helpers accept `--evidence /absolute/new/directory` and refuse an existing directory. Evidence is outside `target/` and survives resource cleanup and `cargo clean`. Headless PNGs use lossless adaptive filtering and balanced compression at full resolution. Both helpers keep the newest eight completed runs per default evidence folder; `--keep-runs N` overrides this and `--keep-runs 0` disables pruning. Active/incomplete runs, failed resource cleanup, explicit evidence paths, and runs containing `.keep` are preserved. Pin or export evidence referenced by durable proof notes before it ages out. Use `python3 scripts/verification_evidence.py --dry-run` to preview existing-run pruning and omit `--dry-run` to apply it. Native artifacts include:

- `doctor-base.json` and `doctor-instance.json`: binary, daemon, process/window and fixture identity.
- `actions.jsonl`: timestamped user keys, typing, control labels and observations.
- Numbered PNGs and adjacent OCR text: real native screenshots, including failed attempts.
- `application.log`, fixture launch/cleanup logs, and feature-specific side-effect JSON.
- `result.json`: feature, status, error and fixture project; `cleanup.json`: owned process termination and project removal result.

For terminal proof, require both visible command output and an independent read-only check of a newly created file **inside the disposable container**, plus nonzero PTY dimensions. Echoed input alone is insufficient. For lifecycle proof, require daemon state/timestamp changes as well as visible UI results. Report the actual run path and any missed entry points. Evidence is local and ignored by Git; retain or export it deliberately when needed for review.

## Cleanup

Headless tests release entity handles before the context closes; GPUI's leak detector checks teardown. The wrapper waits for its owned Cargo/test process and terminates only that process group on timeout/interruption. After writing the result it prunes older completed default-path evidence according to the retention limit. It performs no Docker cleanup because no daemon worker/resources are created. Require `owned_process_exited: true` and overall `status: passed`; a flow that asserts successfully but fails teardown still fails.

The native helper cleans up in `finally` after success or failure. It terminates only its child application PID, validates ownership of every container in its random project, and runs Compose down only for that project. It confirms that no project containers or networks remain and keeps evidence. Require `cleanup.json` to contain `success: true`; a failed cleanup makes the run fail.

The Compose check traps exit and runs down only for its uniquely named project, then verifies that its containers and network are absent. Retained settings and evidence are not removed. The fixture has no host mounts, ports or persistent volumes. A forcibly killed script requires scoped recovery using the project and endpoint in its retained result; never use global prune or another project's down command.

If the helper is forcibly killed, use the retained `doctor-base.json` endpoint and `doctor-instance.json` PID/project to inspect ownership before manual cleanup. Confirm `/proc/<pid>/exe` still matches the recorded binary; never kill by process name. Confirm every matching container has both `dev.ferrydock.fixture=true` and the recorded Compose project label before scoped Compose down. Never run global prune, global `pkill`, or another project's down command. Preserve the evidence and record the recovery.

The per-user `/tmp/ferrydock-native-verification-<uid>.lock` is an advisory lock file; it may remain on disk while unlocked. Do not delete it to bypass an active run.

## Helpers

- [scripts/verify-headless.py](../../../scripts/verify-headless.py): `python3 scripts/verify-headless.py`, default development flow with GPU evidence and owned teardown. `--help` lists optional evidence path and timeout.
- [src/headless_tests.rs](../../../src/headless_tests.rs): input-driven UI flow with the renderer factory, worker fixture, assertions and capture timings.
- [scripts/native.py](scripts/native.py): native front-door harness, doctor, evidence and owned cleanup. `--help` lists the interface.
- [scripts/fixture.compose.yaml](scripts/fixture.compose.yaml): isolated probe with initial/live log markers and an interactive shell.
- Project `scripts/verify-docker.sh`: separate Docker API integration checks. This does not replace native UI proof.
- Project [scripts/verify-compose.sh](../../../scripts/verify-compose.sh): separate Compose association and worker/CLI/daemon checks, with isolated settings and scoped cleanup.
- Project `scripts/measure-native.py`: separate launch/resource measurement. This does not prove feature interaction.

After app changes, use `/maintain-verification-skill` to update entry points, exercise changed flows, and keep coverage claims honest.
