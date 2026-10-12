---
name: verify-ferrydock
description: Verify FerryDock with fast Linux headless GPU screenshots and GPUI input tests, or with a real native window and disposable Docker fixture. Use for development checks, containers/search, logs, embedded terminal, inspect, and container controls. Preserve screenshots, actions, results and scoped cleanup evidence.
---

# Verify FerryDock

Work from the repository root. Read [features/README.md](features/README.md), then the feature being exercised. Use headless GPU verification for the everyday UI loop, Docker integration checks for daemon behavior, and native-window checks for desktop integration. The headless flow uses the real FerryDock view, WGPU renderer and GPUI input dispatch, with a fixture at the production Docker worker channel boundary. Browser automation cannot drive this app.

## Launch

The default development check needs a Linux GPU/driver usable by WGPU, Python 3.11+, and cached Cargo dependencies. It needs neither a display server nor Docker:

```sh
python3 scripts/verify-headless.py
```

The helper removes `DISPLAY` and `WAYLAND_DISPLAY` only from its child environment, enables `headless-tests`, and builds/runs the current source with the locked dependency graph. All GPUI packages share Git revision `43dacc568497c615c55a1ea0f77ab381c0d9865b`; fetch once with `CARGO_NET_GIT_FETCH_WITH_CLI=true cargo fetch --locked` if the cache is missing. It supplies the Linux headless renderer explicitly; GPU initialization failure fails the test, with no no-op renderer fallback. Each run owns a virtual GPUI window/context, fixture channels and a unique evidence directory. It never starts the Docker worker or creates a compositor window. Cold compilation is distinct from warm execution.

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

The headless helper prints its unique evidence directory under `.verification/headless/`. It retains `result.json` (revision, lock hash, command, exit status, timing and cleanup), `headless.json` (checks, dimensions, per-frame render/encoding timings), `actions.json`, `test.log`, and six PNGs. The virtual display currently uses 2x scaling, producing 2360x1640 pixels for a 1180x820 logical view; dimensions are checked against the reported scale. Inspect the initial list, empty-search and terminal frames when reviewing visual changes. Timing includes builds at the outer level; the flow's timing excludes compilation.

Native runs print their unique evidence directory under `.verification/ferrydock/`. Both helpers accept `--evidence /absolute/new/directory` and refuse an existing directory. Evidence is outside `target/` and survives cleanup and `cargo clean`. Native artifacts include:

- `doctor-base.json` and `doctor-instance.json`: binary, daemon, process/window and fixture identity.
- `actions.jsonl`: timestamped user keys, typing, control labels and observations.
- Numbered PNGs and adjacent OCR text: real native screenshots, including failed attempts.
- `application.log`, fixture launch/cleanup logs, and feature-specific side-effect JSON.
- `result.json`: feature, status, error and fixture project; `cleanup.json`: owned process termination and project removal result.

For terminal proof, require both visible command output and an independent read-only check of a newly created file **inside the disposable container**, plus nonzero PTY dimensions. Echoed input alone is insufficient. For lifecycle proof, require daemon state/timestamp changes as well as visible UI results. Report the actual run path and any missed entry points. Evidence is local and ignored by Git; retain or export it deliberately when needed for review.

## Cleanup

Headless tests release entity handles before the context closes; GPUI's leak detector checks teardown. The wrapper waits for its owned Cargo/test process and terminates only that process group on timeout/interruption. It never removes evidence or performs Docker cleanup because no daemon worker/resources are created. Require `owned_process_exited: true` and overall `status: passed`; a flow that asserts successfully but fails teardown still fails.

The native helper cleans up in `finally` after success or failure. It terminates only its child application PID, validates ownership of every container in its random project, and runs Compose down only for that project. It confirms that no project containers or networks remain and keeps evidence. Require `cleanup.json` to contain `success: true`; a failed cleanup makes the run fail.

If the helper is forcibly killed, use the retained `doctor-base.json` endpoint and `doctor-instance.json` PID/project to inspect ownership before manual cleanup. Confirm `/proc/<pid>/exe` still matches the recorded binary; never kill by process name. Confirm every matching container has both `dev.ferrydock.fixture=true` and the recorded Compose project label before scoped Compose down. Never run global prune, global `pkill`, or another project's down command. Preserve the evidence and record the recovery.

The per-user `/tmp/ferrydock-native-verification-<uid>.lock` is an advisory lock file; it may remain on disk while unlocked. Do not delete it to bypass an active run.

## Helpers

- [scripts/verify-headless.py](../../../scripts/verify-headless.py): `python3 scripts/verify-headless.py`, default development flow with GPU evidence and owned teardown. `--help` lists optional evidence path and timeout.
- [src/headless_tests.rs](../../../src/headless_tests.rs): input-driven UI flow with the renderer factory, worker fixture, assertions and capture timings.
- [scripts/native.py](scripts/native.py): native front-door harness, doctor, evidence and owned cleanup. `--help` lists the interface.
- [scripts/fixture.compose.yaml](scripts/fixture.compose.yaml): isolated probe with initial/live log markers and an interactive shell.
- Project `scripts/verify-docker.sh`: separate Docker API integration checks. This does not replace native UI proof.
- Project `scripts/measure-native.py`: separate launch/resource measurement. This does not prove feature interaction.

After app changes, use `/maintain-verification-skill` to update entry points, exercise changed flows, and keep coverage claims honest.
