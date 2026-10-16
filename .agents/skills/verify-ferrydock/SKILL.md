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

The Linux desktop needs an accessible X11 display, Docker Engine with a local Unix socket, Docker Compose, `xdotool`, ImageMagick's `magick`, Tesseract with English data, and Python 3 with Pillow. GPUI's system build dependencies are listed in [the development guide](../../../docs/development.md#build-on-linux). Commands use `:0` as an example; use your actual display. The application normally selects native Wayland when available. This X11-specific harness removes `WAYLAND_DISPLAY` only from its child environment so its window ownership and input checks still work; it does not prove native Wayland integration.

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

The appearance flow also uses the Kit Settings page, changes FerryDock/Dark/Light through visible controls, resets the default, checks isolated preference persistence, and verifies Light log/terminal glyphs and session/focus preservation. Container/project popup menus and right-click Inspect, the running-only Checkbox, shell RadioGroup disabled state, and searchable service Select are input-driven. See [features/appearance.md](features/appearance.md).

The Kit control flow additionally traverses to a real Button with Tab and activates
it through Enter/Space, checks loading/disabled action protection, clicks both
TabBars by their native scoped integer IDs, and drags Resizable using measured
pane bounds. Inspect tests use fixture-only JSON, count syntax-colored pixels
inside actual editor text bounds, reject editing, and drive the built-in search.
Notifications are in-app; their close control is revealed by hover. The harness
commits hover hitboxes before a click and settles both painted animation time and
the simulated lifecycle clock before asserting dismissal or persistence.

The same headless flow also clicks a project heading, edits the Compose association form through keyboard events, and asserts ordered files, environment files and profiles. It checks disabled unassociated/conflicting controls, outgoing Start/Stop/Restart/Rebuild requests, rebuild toolbar/menu actions, progress/success, project/container conflict guards, and build failure/output retained after leaving and returning using fixture events. Inspect the association, operation live/failure and rebuild live/failure/menu/completed screenshots when this feature changes. No Compose subprocess is launched by headless tests.

Project headings default to the combined Logs tab. The headless flow proves the project stream request without association, source-prefixed records, service-filter rendered rows, follow/pause, copy feedback, recreated-member rendering and return-to-container routing. Inspect the all-services, filtered and recreated-member frames. Copy text formatting has a unit check; the headless action does not prove the operating system clipboard. See [features/project-logs.md](features/project-logs.md).

Busy-log verification fills the retained 1,500-line history and feeds 10,000 additional records in batches. Require `log-performance.json` to show fewer than 80 rendered rows, the retained history bound, newest-line visibility, `follow_reached_tail: true` and `controls_respond_after_flood: true`. Inspect the project-logs-busy and project-logs-batched-busy frames. These measurements exercise rendering and fixture-channel ingestion, not native CPU or real Docker throughput. The container list and both log lists are virtualized; horizontal scrolling intentionally keeps long log records on one line.

ANSI log verification uses the existing headless flow: split colour escapes in container output, green project payload with independently coloured source labels and neutral other-service payload, and coloured Compose output replacing a partial `Starting` line with `Started` through CR/erase-in-line. Inspect the containers-and-logs, project-logs-all, project-logs-filtered and compose-operation-live frames. Assertions check actual colour pixels within the corresponding rendered log-row bounds, excluding similarly coloured controls; they also require readable copy text without escape bytes. Unit checks cover resets, RGB/indexed styles, per-container decoder state, split UTF-8, progress editing, Docker timestamp removal, preserved application dates and bounds. This is a log-record renderer, not full-screen terminal emulation; native clipboard and every visual SGR attribute remain outside the headless proof.

For development-build log performance, intentionally build with `cargo build --locked`, then run `python3 scripts/verify-log-performance.py`. Use `--binary target/release/ferrydock` after a release build to measure shipping performance. The default `--backend auto` selects native Wayland when `WAYLAND_DISPLAY` is set, otherwise X11; `--backend wayland` requires Wayland and `--backend x11 --display :0` enables an explicit XWayland comparison. A missing Wayland prerequisite fails instead of silently switching backends. The demonstrated Wayland adapter requires KDE/KWin, `kdotool`, `qdbus6`, Spectacle, `ydotool`, `ydotoold`, and access to `/dev/uinput`. Other compositors need their own adapter. X11 requires `xdotool` and ImageMagick; both use Tesseract.

This separate check owns its window and ten uniquely labelled disposable containers producing roughly 500 lines/s. It takes desktop focus, uses the native-input lock, health-checks the selected backend and daemon, validates PID/executable/window/fixture ownership, checks the selected project header and live output, measures CPU/RSS, types a no-match search under load, and cleans up its resources. Wayland removes `DISPLAY` from the child environment and uses a temporary keyboard daemon/socket; input and active-window capture require the owned PID and focus. `doctor.json`, `input-instance.json`, and `input-cleanup.json` retain health and input ownership/cleanup evidence. Require `result.json` status passed, `controls_respond_under_load`, `owned_process_exited`, `cleanup_success`, and both `input_cleanup` flags true. The actual backend and input/capture methods are recorded. Capture/OCR and typing timings include backend-specific overhead; CPU is an observation on this workstation, not a portable threshold. Clipboard integration is outside this check. Do not run alongside another native-input session.

Add `--ansi` to that helper to emit real green UTF-8 fixture records. Require `ansi_fixture: true` in the result and inspect the retained native log screenshot for coloured payloads/neutral metadata with no literal escape codes. This exercises the production Docker stream through the owned native window under load; do not claim native filter/pause/copy controls were driven.

For real Compose process behavior, run `bash scripts/verify-compose.sh`. This creates a unique `ferrydock-compose-verify-<timestamp>-<pid>` two-service project with base/override files, an environment file, a profile and an isolated build context under its evidence directory. It pins the current local Unix endpoint, isolates `XDG_CONFIG_HOME`, drives production worker commands, checks daemon effects and streamed CLI output, proves changed build inputs produce a new image/container with updated contents, and checks build failure leaves services running. It removes only that project's containers, network and uniquely labelled built images on exit; build cache and the base image remain. It deliberately supplies conflicting ambient Compose variables to prove that saved settings win. It does not drive native GPUI input. Read [features/compose.md](features/compose.md) for the combined proof boundary.

For native desktop integration, choose one mapped flow; each invocation owns a new session and cleans it up:

The embedded-terminal flow is the exercised baseline. Mouse-dependent flows
currently stop safely at hover calibration on the observed multi-monitor
desktop; Inspect has headless proof but its native helper has not been run. Read
[the feature map](features/README.md) before selecting broader coverage.
If present, `.verification/VERIFICATION.md` at the repository root contains
local run history; it is ignored by Git and is not required to use the skill.
Do not treat these additional commands as passing checks until the native display path has been demonstrated.

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

For Docker lifecycle annotations, run `python3 scripts/verify-headless.py --flow lifecycle` and inspect the container, refreshed, project, reconnecting/recovered and Light frames. Pair this with `bash scripts/verify-docker.sh docker_external_lifecycle_markers` for real external CLI stop/start/restart events on the owned fixture and `cargo test --lib lifecycle::tests` for interrupted event-stream replay/cancellation. These do not restart the shared Docker daemon or prove native compositor/clipboard integration. See [Logs](features/logs.md).

For AppImage updater controls, run `python3 scripts/verify-headless.py --flow updates`.
Inspect the Settings, progress, ready, restart review, applying, error and
non-AppImage screenshots. This drives fixture worker requests through GPUI input;
it does not install a public release or exit a desktop process. Pair it with
`cargo test --lib updater::tests -- --test-threads=1` for loopback HTTP, checksum,
atomic replacement and rollback proof. See [features/updates.md](features/updates.md).

The headless helper prints its unique evidence directory under `.verification/headless/`. It retains `result.json` (engine package/version, lock hash, command, exit status, timing and cleanup), `headless.json` (checks, dimensions, per-frame render/encoding timings), `log-performance.json`, `actions.json`, `test.log`, and all PNGs listed by the flow. The virtual display currently uses 2x scaling, producing 2360x1640 pixels for a 1180x820 logical view; dimensions are checked against the reported scale. Inspect the initial list, empty-search, Kit Input editing, log scrollbars, resized dock, Inspect JSON/search, notifications, terminal, project Logs and Compose operation frames when reviewing visual changes. Timing includes builds at the outer level; the flow's timing excludes compilation.

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

The Compose check traps exit and runs down only for its uniquely named project, then removes images carrying that run's unique build label, including superseded images, and verifies that its containers, network and labelled images are absent. Retained settings, build inputs and evidence are not removed. The fixture has no host mounts, ports or persistent volumes. `result.json` is written only after the integration test succeeds; a failed launch, failed test or forcibly killed script may leave no result. For interrupted-run recovery, use a successful result when available; otherwise recover the project from retained launch/test logs and independently establish the original pinned endpoint and fixture ownership before scoped cleanup. If that identity cannot be established, report cleanup as blocked. Never use global prune or another project's down command.

If the helper is forcibly killed, use the retained `doctor-base.json` endpoint and `doctor-instance.json` PID/project to inspect ownership before manual cleanup. Confirm `/proc/<pid>/exe` still matches the recorded binary; never kill by process name. Confirm every matching container has both `dev.ferrydock.fixture=true` and the recorded Compose project label before scoped Compose down. Never run global prune, global `pkill`, or another project's down command. Preserve the evidence and record the recovery.

For interrupted Wayland performance runs, the owned keyboard PID/socket are in `input-instance.json`. Validate that PID's executable and command line against the recorded `ydotoold` socket before terminating it; never stop another input daemon. Remove only that run's socket after its daemon has exited, and preserve the evidence.

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
