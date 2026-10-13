# FerryDock verification

Date: 2026-10-12. Scope: FerryDock 0.2.1 ANSI log rendering and the preceding
0.2.0 Compose project controls, combined logs and busy-log performance.
Earlier Linux milestone and desktop evidence is retained below.

## ANSI log rendering after the 0.2.0 commit

Work started after release commit `febf1b8` and tag `v0.2.0` were committed and
pushed. These changes are prepared for 0.2.1 and are not part of that tag.
Container, project and Compose operation output now use cached ANSI
styles from Alacritty's VTE processor. Standard/bright, indexed and RGB colours
and common text attributes are preserved; source labels/timestamps remain
neutral, and copied text omits escape sequences. Project decoders are scoped
to container IDs, including replicas of the same service. Container and
operation buffers handle partial output and basic CR/backspace/erase-in-line
progress changes, while protecting Docker's injected timestamp prefix from
those edits. This does not turn the log list into a full terminal screen.

Sixteen unit tests and all-target/all-feature Clippy passed. Unit checks cover
split escapes and UTF-8, resets, RGB/indexed colour decoding, attributes,
discarded OSC wrappers, progress replacement, per-container state, clean copy,
line/history/style bounds and expired-source cleanup. The unchanged raw Docker
transport still supplies original timestamps to replay suppression.

The input-driven headless flow passed in **9.839 s including build**, retaining
14 PNGs with owned-process teardown. It checks actual pixels within rendered
log rows for container/project/operation colours, neutral metadata and reset
text, plus replacement of `Starting` with `Started` through CR/erase-in-line
and preservation of Docker timestamps during container progress updates.
An initial exact internal-colour scene assertion failed despite visible green
text; it was replaced with scoped screenshot pixel checks, not a bypass of
colour verification. Luna high inspected frames 01/07/08/11/14 and confirmed
the visible styling and lack of literal escape-code artifacts or pane overlap:
[result](.verification/headless/20261013T014029Z-517bf81c/result.json),
[container logs](.verification/headless/20261013T014029Z-517bf81c/01-containers-and-logs.png),
[project logs](.verification/headless/20261013T014029Z-517bf81c/07-project-logs-all.png),
[operation progress](.verification/headless/20261013T014029Z-517bf81c/11-compose-operation-live.png).
Busy history still renders **17 rows**, retains 1,500 records and reaches the
latest line; the 10,000-record fixture processed in **0.166 s** and controls
responded. The busy frame's synthetic fixture has no production status event
after refreshing, so its `Connecting…` footer is not live-daemon status proof:
[performance](.verification/headless/20261013T014029Z-517bf81c/log-performance.json).
This run is pinned with `.keep`.

A freshly built release was tested with `verify-log-performance.py --ansi`
against ten owned disposable containers producing nominally 500 coloured UTF-8
records/s. The real native X11/XWayland screenshot shows green payloads and
neutral metadata, with no literal SGR codes. The five-second sample measured
**9.2% CPU and 71.9 MiB RSS**, comparable to the preceding 9.4% plain-record
sample rather than a claimed speedup. Search remained responsive; the visible
live-output capture bound was 0.363 s and the completed search capture bound
was 1.166 s including typing, with OCR time reported separately:
[native result](.verification/log-performance/20261013T014129Z-a532d541/result.json),
[coloured Docker logs](.verification/log-performance/20261013T014129Z-a532d541/busy-logs-after-sample.png).
Owned app PID 1084573 exited, and fixture `ferrydock-log-perf-3ba2f73c` plus its
network were removed. No native Wayland input, OS clipboard, full-screen log
controls or every SGR visual attribute is claimed by this check.

## 0.2.0 release checks

The 0.2.0 manifest and lockfile agree, and user-facing release notes are in
[docs/releases/v0.2.0.md](docs/releases/v0.2.0.md). Formatting, all-target and
all-feature Clippy with warnings denied, all ten library tests, Python helper
syntax, shell helper syntax, and the locked shipping release build passed.

The current headless GPU/input flow passed with owned-process teardown and
14 PNGs in **15.952 s including build**. The initial list, empty search,
terminal, combined/filtered/recreated project logs, association form, live and
failed Compose operations, and both busy-log frames were visually inspected.
The 10,000-line burst retained 1,500 lines and rendered 17 rows; assertions
confirmed follow at the tail and responding controls. This run is pinned:
[result](.verification/headless/20261013T010053Z-fcb7b5ee/result.json),
[performance](.verification/headless/20261013T010053Z-fcb7b5ee/log-performance.json).

The real Docker integration test passed discovery, logs, Stop/Start/Restart,
snapshot reconciliation, interactive shell/PTY resize, full-screen vi editing,
shell exit, and the missing-Bash error. Its disposable container and network
were removed. The separate Compose worker test passed all 14 checks, including
saved configuration, override/environment/profile handling, lifecycle effects,
failure output, restart/recreation log attachment, and a 10,000-record burst
without lost lines. Its two containers and network were removed:
[result](.verification/compose/20261013T010139Z-ZpzrUC/result.json),
[cleanup](.verification/compose/20261013T010139Z-ZpzrUC/cleanup.log).

These checks establish headless UI rendering/input and real worker/daemon
behavior separately. This release pass did not repeat native Wayland,
clipboard, or desktop Compose clicks. Ubuntu package installation and AppImage
checks run in the tag-triggered release workflow; the local workstation is
CachyOS, so its binary is not used as the published Ubuntu build.

## Busy project-log performance regression

The reported all-services slowdown came from laying out every retained log
record on each redraw and sending records individually to the UI. Both log
views and the container list now render through virtual lists. The production
project worker batches records for up to 50 ms, bounded to 256 lines/64 KiB
(one oversized bounded record may travel alone); the UI drains events with an
8 ms per-poll budget. Follow uses the virtual list's current tail directly.
Service names are cached in the bounded history. GPUI and Taffy are optimized
in development builds after native UI-thread stacks identified layout work
as a remaining hotspot.

The instrumented headless baseline laid out **1,500 rows** and took **9.062 s**
to ingest the 1,500-record fixture. Final source lays out **17 rows** (visible
rows plus measurement work), ingests that fixture in **0.670 s**, and processes
the 10,000-record batched fixture in **0.027 s** while keeping 1,500 records.
Assertions require newest-line bounds, follow at the tail and responding
controls. These are fixture-channel/layout measurements, not daemon throughput
or screenshot GPU readback benchmarks:
[baseline](.verification/headless/20261013T000723Z-c9834b71/log-performance.json),
[final metrics](.verification/headless/20261013T004135Z-587fa951/log-performance.json),
[final result](.verification/headless/20261013T004135Z-587fa951/result.json).
The final warm check retained 14 PNGs and passed in **12.242 s including build**
(8.14 s test execution). Luna high reviewed the preceding equivalent layout
at original resolution, including all/filtered and busy-tail frames, and
found no pane overlap. Long records intentionally scroll horizontally:
[Luna run](.verification/headless/20261013T003645Z-eb289b92/result.json).
These headless runs are pinned with `.keep`.

The production Compose check received **10,000 real records in 40 batches**
in **0.368 s**, verified the history transport without lost records, and passed
existing association/lifecycle/recreation checks. Both fixture containers and
their project network were removed:
[result](.verification/compose/20261013T001636Z-Guck4o/result.json),
[cleanup](.verification/compose/20261013T001636Z-Guck4o/cleanup.log).

The native performance helper owns ten disposable containers generating
nominally 500 lines/s and a PID/executable/title-verified X11/XWayland window.
An early development run still reproduced **95.39% CPU** after the first
virtual-log change. Final development samples measured **23–35.2%**, with
search confirmed under load; the freshly rebuilt **release measured 9.4% CPU
and 65.9 MiB RSS** over its five-second sample. Build profiles differ, so the
release number is not a same-profile before/after comparison. The release
obtained a selected-project/live-tail capture within **1.793 s**, and captured
the completed no-match search within **1.123 s including synthetic typing**.
OCR takes additional time and is reported separately. These are capture upper
bounds and workstation observations, not portable latency/CPU thresholds:
[development baseline](.verification/log-performance/20261013T002416Z-36fbc835/result.json),
[final development](.verification/log-performance/20261013T004201Z-b9ef3e1c/result.json),
[release](.verification/log-performance/20261013T004540Z-b882a1e4/result.json),
[release search](.verification/log-performance/20261013T004540Z-b882a1e4/responsive-search.png).
The release fixture `ferrydock-log-perf-e3223f28`, its network and app PID
864254 were removed; both owned-process exit and cleanup flags passed.
No native Wayland or clipboard interaction is claimed by this helper.

Ten unit tests, all-target/all-feature Clippy with warnings denied, formatting,
and release compilation passed. Ctrl+4 now opens the selected container's
Compose project logs from the main-list focus. The new stress flow, native
helper and proof boundaries are documented in the project verification skill.

## Combined Compose project logs

Project workspaces now have Logs and Operations tabs. Logs use Docker labels,
so no Compose association is needed. Lines retain timestamps and service/
container prefixes; service filters and copy use the shared bounded history.
Follow/pause affects scrolling while collection continues. Each stream starts
with the last 200 lines and retries from a timestamp cursor; the worker watches
membership to attach replacement container IDs. The global history is capped
at 1,500 lines, merged as received rather than globally sorted by timestamp.

Luna passed the expanded headless flow in **17.811 seconds including compilation**,
retaining 12 screenshots. It checked combined/filtered/recreated project rows,
follow/pause and copy feedback, routing back to container logs, and coexistence
with the Compose operation/configuration workspace:
[result](.verification/headless/20261012T235355Z-67ea0335/result.json),
[combined logs](.verification/headless/20261012T235355Z-67ea0335/07-project-logs-all.png),
[service filter](.verification/headless/20261012T235355Z-67ea0335/08-project-logs-filtered.png),
[recreated member](.verification/headless/20261012T235355Z-67ea0335/09-project-logs-recreated.png).
A suspected row overlap was independently inspected and reassessed by Luna:
the partial row ends at the scroll viewport boundary and does not paint into
the dock. No concrete visual defect remained.

The final real Compose fixture passed in **5.38 seconds of test execution**. Both
services' stdout/stderr were attributed to the correct source, logs resumed
after Stop/Start/Restart (requiring timestamps after both services restarted),
and a force-recreated service's new container ID
emitted its startup marker without reopening the project stream. Existing
configuration/action/error checks also passed:
[result](.verification/compose/20261012T235850Z-BOKLV8/result.json),
[cleanup](.verification/compose/20261012T235850Z-BOKLV8/cleanup.log).
Project `ferrydock-compose-verify-1791849530-532402` was removed and the script
confirmed its containers/network were absent. Settings remained isolated.
Nine unit tests and all-target/all-feature Clippy passed. Timestamp replay
counts, late live lines, bounded mixed-service history and filtered copy text
have unit coverage. Native clipboard/compositor integration remains separate.
The existing Docker/container-log/terminal regression passed in 21.96 seconds;
its fixture `ferrydock-verify-1791849365-514322` was also removed.

## Compose project management

Project headings now open a bottom-dock workspace for configuration association
and Start/Stop/Restart. Labels prefill file suggestions, but an explicitly saved
association enables operations. Associations retain ordered files, environment
files and profiles, keyed by Docker endpoint and project name. Start uses
`up --detach --no-build`; Stop/Restart and container actions share project
conflict guards. The dock retains the latest output and result per project.

Luna passed the expanded headless flow, including keyboard form entry,
unassociated/conflicting action guards, all three outgoing operation requests,
live output, and retained failure. It inspected the new frames and reported no
visible defects. The final run retained nine PNGs in **12.649 seconds including
compilation**:
[result](.verification/headless/20261012T232843Z-b05f1fc1/result.json),
[association](.verification/headless/20261012T232843Z-b05f1fc1/07-compose-association.png),
[live output](.verification/headless/20261012T232843Z-b05f1fc1/08-compose-operation-live.png),
[failure](.verification/headless/20261012T232843Z-b05f1fc1/09-compose-operation-failed.png).

`bash scripts/verify-compose.sh` passed the production worker and real Compose
CLI against a uniquely owned two-service fixture. It verified label discovery,
association required before operation, persistent endpoint-scoped settings,
file override order, an environment file, a profiled service, Stop/Start/Restart
daemon state and restart timestamp, streamed output, ignored hostile ambient
Compose overrides, a real invalid-YAML CLI failure and streamed error output,
and rejected invalid associations without losing settings.
The final test itself took **3.11 seconds**:
[result](.verification/compose/20261012T232834Z-P4ydvM/result.json),
[test log](.verification/compose/20261012T232834Z-P4ydvM/test.log),
[cleanup](.verification/compose/20261012T232834Z-P4ydvM/cleanup.log).
Fixture project `ferrydock-compose-verify-1791847714-345030` was removed; the
script confirmed no project containers or networks remained. Settings were
isolated inside the retained evidence directory. No user project was changed.

Seven unit tests, formatting, and all-target/all-feature Clippy with warnings
denied passed. The existing Docker/terminal regression also passed in 21.93
seconds, including container lifecycle, PTY/ANSI, vi editing, shell exit and
missing Bash; its fixture `ferrydock-verify-1791847594-332917` was removed.
The new Compose script is wired into CI, but CI execution is not
claimed here. Native desktop Compose clicks, cancellation UI, Rebuild and Down
are outside this slice. Closing/disconnecting stops the tracked process; it
does not roll back Docker effects. The previous release binary is not this
source's verification artifact.

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
