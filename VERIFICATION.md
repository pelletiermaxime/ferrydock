# FerryDock verification

Date: 2026-10-12. Scope: the first runnable Linux milestone in `PLAN.md`,
including native Wayland/X11 selection, its GPUI Community Edition Git upgrade
and Linux headless GPU verification. Earlier evidence is retained below.

## Native Wayland and X11 selection

Both backends are explicitly enabled on the pinned GPUI crates. FerryDock
accepts either `WAYLAND_DISPLAY` or `DISPLAY` and leaves those variables intact.
GPUI prefers native Wayland when its display variable is nonempty, otherwise
selects X11. This is selection at startup, not an automatic retry of X11 after
a Wayland connection failure. The X11-specific native verifier now removes
`WAYLAND_DISPLAY` only in its child environment.

The incremental debug build passed a native Wayland smoke check on the KDE
desktop with `DISPLAY` removed. The owned process received keyboard focus and
committed Wayland surfaces; its active-window screenshot was inspected and
showed the FerryDock UI. It used an intentionally unavailable Docker socket,
so the visible disconnected state is expected and no containers were affected.
The process was terminated after capture. Evidence:
[Wayland result](.verification/wayland/20261012T214414Z-67466d6e/result.json),
[window screenshot](.verification/wayland/20261012T214414Z-67466d6e/window.png),
[Wayland protocol log](.verification/wayland/20261012T214414Z-67466d6e/application.log).

The same binary also created a visible X11 window with `WAYLAND_DISPLAY`
removed, matched by its owned PID and exact title, and was then terminated:
[X11 result](.verification/wayland/20261012T214414Z-67466d6e/x11-result.json).
The missing-display guard still exits with an actionable error.

Luna reran the headless flow successfully in **10.442 seconds**, retaining six
screenshots and inspecting the initial list, empty search and terminal frames:
[result](.verification/headless/20261012T214342Z-6c61d1a7/result.json).
Formatting, all-target/all-feature Clippy with warnings denied, and the five
existing unit tests passed. The opt-in Docker integration test was not rerun
for this startup-only change. The native launch proof uses the development
binary; the previous release binary is not rebuilt by this check.

Wayland keyboard typing, resizing, clipboard, IME and Docker-backed terminal
interaction remain unverified. Headless input proof does not establish those
desktop behaviors. The earlier X11/Docker evidence below retains its original
scope and binary identity.

## Historical GPUI-CE Git upgrade and headless GPU verification

All 19 GPUI packages are pinned to Git revision
`43dacc568497c615c55a1ea0f77ab381c0d9865b`. The application uses that revision's
window-options builder and updated text-shaping API. No upstream code was
vendored or patched. Headless support is enabled only with `headless-tests`;
normal release builds retain X11/XWayland and the real Docker worker.

The test explicitly supplies GPUI-CE's Linux `WgpuHeadlessRenderer` to
`HeadlessAppContext`. It renders the actual FerryDock view, uses real font
shaping and GPUI keyboard/pointer dispatch, and checks outgoing commands at a
fixture Docker channel boundary. It never starts the daemon worker, creates
containers/networks, or opens a compositor window.

```sh
python3 scripts/verify-headless.py
```

The final warm run passed in **7.333 seconds wall-clock**, including Cargo's
check; **6.804 seconds** were spent in the render/input flow. Six 2360x1640
PNGs represent a 1180x820 logical viewport at GPUI's 2x virtual-display scale.
Stable image/compression dependencies are optimized in the development profile;
FerryDock itself stays unoptimized for quick incremental builds. These are
single-run observations, not CI/performance guarantees. Cold compilation and
concurrent builds have separate costs.

Evidence: [result](.verification/headless/20261012T205201Z-f9c9f7d1/result.json),
[flow/frame timings](.verification/headless/20261012T205201Z-f9c9f7d1/headless.json),
[actions](.verification/headless/20261012T205201Z-f9c9f7d1/actions.json),
and [terminal frame](.verification/headless/20261012T205201Z-f9c9f7d1/06-terminal-rendered.png).
The process had neither display variable, GPUI teardown/leak checks passed,
and artifacts survived cleanup outside the Cargo target directory.

The flow verifies visible fixture rows, search/empty/restored row rendering,
keyboard selection, log-follow pointer toggles, a Stop request, Enter to request
a terminal, ANSI terminal output and exact keyboard-to-terminal bytes. It checks
painted glyphs/nonblank images and image changes, but has no golden-image
baseline. Real Docker effects, shell execution, clipboard, compositor input,
other entry points and other platforms remain separate proof boundaries.

Passed upgrade checks:

- Formatting and all-target/all-feature Clippy with warnings denied.
- Five existing model/terminal unit tests; the headless GPU test passed separately.
- Normal release build and the expected missing-display error.
- `bash scripts/verify-docker.sh --release`: real fixture discovery/logs,
  lifecycle actions, interactive shell/PTY/ANSI, vi edit, exit and missing Bash.
  The script accepts Cargo flags to reuse a release build; its cleanup stayed scoped.
- The native embedded-terminal flow using the upgraded release:
  [result](.verification/ferrydock/20261012T205704Z-db8e6a18/result.json),
  [side effect](.verification/ferrydock/20261012T205704Z-db8e6a18/side-effect-after.json),
  [cleanup](.verification/ferrydock/20261012T205704Z-db8e6a18/cleanup.json).
  A real shell created the independently observed marker file and reported
  13x128 PTY dimensions. Seven screenshots remain after owned cleanup.

Release SHA-256 for that verification:
`94146203b7d106bfcebf177b3ec6cb16175f6e345300dc7cd63e7a0f1b7f7797`.

## Historical registry GPUI-CE migration

The previous CE release used pinned registry packages `gpui-ce` 0.2.2 and
`gpui_ce_platform` 0.1.0. Linux rendering uses `gpui_ce_wgpu` 0.1.0 and wgpu
29.0.4. That lockfile contained no Blade packages or Git dependencies. The direct
libc compatibility pin was removed with the old GPUI dependency chain.

Startup uses `gpui_platform::application()`. The published CE platform enables
Wayland transitively and prefers it when `WAYLAND_DISPLAY` is present, so this
Linux build clears that variable in its own process before creating any
threads. This preserves the existing X11/XWayland path without asking users
to change their desktop environment. Native Wayland remains unverified.

Passed checks:

- `cargo check --all-targets --locked --offline`.
- `cargo fmt --all --check` and
  `cargo clippy --all-targets --locked --offline -- -D warnings`.
- `cargo build --release --locked --offline`.
- `cargo test --release --all-targets --locked --offline`: all five unit tests
  passed; the Docker test is opt-in and was then executed separately.
- The existing Docker integration test passed in release mode against an owned
  `verify/compose.yaml` project. It proved real logs, stop/start/restart,
  interactive shell I/O, PTY resizing, ANSI parsing, a full-screen `vi` edit,
  normal shell exit, and missing-Bash handling. Its fixture was removed.
- Missing `DISPLAY` produces the expected actionable error and exit status 1.

The real-window terminal flow passed through the project-local native helper:

```sh
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature terminal --display :0
```

Evidence: [native result](.verification/ferrydock/20261012T200150Z-50c7b4cf/result.json),
[confirmed terminal output](.verification/ferrydock/20261012T200150Z-50c7b4cf/006-04-confirmed-terminal-output.png),
[independent fixture side effect](.verification/ferrydock/20261012T200150Z-50c7b4cf/side-effect-after.json),
and [cleanup](.verification/ferrydock/20261012T200150Z-50c7b4cf/cleanup.json).
The screenshot and read-only Docker observation confirmed command execution
and a 13-row × 128-column PTY, followed by visible shell exit. The owned
application PID and fixture containers/networks were independently confirmed
absent after cleanup. All seven screenshots were retained.

Release SHA-256:
`88f14728cae43541125a8736113f9ae291d6be9df9b04435c20552f86d509170`.

An initial native attempt before explicitly selecting X11 found no owned X11
window with CE's automatic platform selection. Its process and fixture were
cleaned up; the successful run above used the corrected startup path. This
does not establish whether native Wayland rendering works.

### Memory comparison

The same `scripts/measure-native.py` helper measured the preserved original
release and final migrated release on display `:0`, with the default Docker
context and initial log view. GPU/platform allocations are included in RSS.

| Measurement | Upstream GPUI / Blade | GPUI-CE / wgpu |
| --- | ---: | ---: |
| Settled resident memory | 459.5 MiB | 215.6 MiB |
| Stripped release binary | 18.93 MiB | 29.03 MiB |

This is a reduction of 243.9 MiB, approximately 53%, in this single-run
comparison. An earlier CE X11 sample measured 211.2 MiB. The final CE sample
reached a visible window in 0.183 seconds and used 1.4% process CPU over five
seconds. Startup, CPU and memory vary with caches, driver state and workload;
these observations are not a controlled benchmark or a guarantee.

The migrated build's native proof covers discovery, filtering, keyboard
selection, basic terminal I/O, initial dimensions and exit. Clipboard,
selection, mouse resizing/actions, full-screen native terminal rendering,
native Wayland, macOS and Windows remain outside this migration's UI proof.
The Docker integration test verifies transport/emulation independently.

## Original upstream GPUI build and automated checks

- Published upstream GPUI 0.2.2, Bollard 0.21.1, Alacritty terminal 0.26.0.
- Rust 1.99.0 on Linux. Cargo lockfile is included.
- Formatting checked with `cargo fmt --all --check`.
- Clippy checked across all targets with warnings denied.
- Five unit tests cover split UTF-8, log bounds, Compose replicas/filtering,
  separation of a project literally named `Standalone`, terminal parser state,
  xterm key sequences, terminal replies, and bracketed paste.
- The opt-in Docker integration test passed against a newly created, uniquely
  named Compose fixture using `alpine:3.23`.

The integration check exercised real Compose-label discovery, container logs,
stop/start/restart, an interactive shell, a 100-column × 30-row PTY resize,
ANSI parsing, a full-screen `vi` edit inside the fixture, normal shell exit,
and the failure when Bash is unavailable. Fixture containers and networks were
removed by the verification script's exit trap. Alpine remains cached locally.

The build reports a future-compatibility notice in upstream
`proc-macro-error2`; current compilation and the application Clippy checks pass.
The libc dependency is pinned to 0.2.186 for the published GPUI dependency
chain's `xattr` compatibility, without modifying or vendoring upstream code.

## Original upstream GPUI native interaction

The optimized application was launched on the local KDE desktop through X11 /
XWayland. A separate dedicated Compose fixture was used for native interaction.

Verified in the actual GPUI window:

- Native window rendering and the container/Compose list.
- Search using Ctrl+F, typed input, and keyboard row selection.
- Detail tabs using the keyboard.
- Opening the selected fixture's embedded shell using Enter.
- Typing `echo FerryDock terminal is alive; stty size` into that shell, and
  seeing both its output and the PTY dimensions rendered in FerryDock.
- The final X11-only build launched with the desktop's normal Wayland display
  variables present, through XWayland. Its expanded dock reported 13 rows ×
  128 columns in the native shell, demonstrating sizing from the GPUI view.
- Launching without `DISPLAY` exits with an actionable message rather than
  silently creating a headless application.

The shipping window is recorded in
[the native terminal screenshot](docs/screenshots/ferrydock-terminal.png).

The terminal transport and emulator were also tested with full-screen `vi`
through the Docker integration test. That test proves the transport/emulator;
it is separate from native window interaction.

Native clipboard round trips, drag selection, mouse-driven dock resizing, and
the container action buttons' physical mouse interactions were not fully
verified. Container action API behavior was verified by the integration check.
macOS, Windows, clean Ubuntu builds, and native Wayland are outside this proof.

The published GPUI version did not produce a visible native Wayland window
on this host. This milestone therefore compiles the verified X11 backend and
uses XWayland on a Wayland desktop. It does not need a manual environment
override when launched from a normal desktop terminal with `DISPLAY` set.

## Original upstream GPUI performance baseline

One warm X11 launch was measured with `scripts/measure-native.py`. Its window
lookup requires both the new process PID and the FerryDock title, avoiding
matches against an already open application.

The final shipping measurement was 18.93 MiB for the stripped optimized binary,
0.358 seconds to a visible window, approximately 305.5 MiB RSS after settling,
and 1.0% process CPU over one five-second sample. This includes the GPU/platform driver allocations and the
default Docker context/initial log view. These are single-run observations,
not guarantees or a memory optimization claim.

The small binary and responsive launch establish an initial baseline. Reducing
resident memory, broader native input testing, and release packaging remain
useful follow-up work.

## Remaining scope

Compose file association, project/service commands and rebuilds, networks,
volumes, and cleanup are the later milestones in `PLAN.md`. The current UI
exposes implemented controls only.
