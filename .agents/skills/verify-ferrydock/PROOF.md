# FerryDock verification execution

## Busy-log performance correction

The all-services regression now has an instrumented before/after flow. Final
source renders 17 rows instead of the retained 1,500, ingests that headless
fixture in 0.670 s instead of 9.062 s, and processes a 10,000-record batched
fixture in 0.027 s while following the latest line. Fourteen PNGs and control/
tail assertions passed in 12.242 s including the final warm build:
[final result](../../../.verification/headless/20261013T004135Z-587fa951/result.json),
[metrics](../../../.verification/headless/20261013T004135Z-587fa951/log-performance.json).
Luna high reviewed the preceding equivalent layout at original resolution,
including all/filtered and busy frames, and found no pane overlap:
[Luna run](../../../.verification/headless/20261013T003645Z-eb289b92/result.json).
Long log records intentionally stay on one horizontally scrollable line.
The headless baseline/final/reviewer runs are pinned with `.keep`.

The production worker received 10,000 real Docker records in 40 bounded batches
in 0.368 s; existing configuration, lifecycle and replacement-member checks
also passed, with scoped cleanup:
[Compose result](../../../.verification/compose/20261013T001636Z-Guck4o/result.json).

The native stress helper proved selected-project live output and search under
nominal 500 lines/s from ten disposable containers in an owned X11/XWayland
window. The rebuilt release measured 9.4% CPU and 65.9 MiB RSS over five seconds;
it captured live-tail output within 1.793 s and the completed no-match search
within 1.123 s including synthetic typing. OCR overhead is separately recorded.
Final development measurements were 23–35.2% CPU, compared with the early
development reproduction at 95.39%; release and development are different
profiles and must not be compared as a same-build benchmark:
[release result](../../../.verification/log-performance/20261013T004540Z-b882a1e4/result.json),
[release search](../../../.verification/log-performance/20261013T004540Z-b882a1e4/responsive-search.png).
Owned PID 864254 exited, and fixture `ferrydock-log-perf-e3223f28` and its network
were removed. Native Wayland and clipboard were not driven by this helper.
Ten unit tests, all-target/all-feature Clippy, formatting and release compilation
passed. Full details and baseline evidence are in project `VERIFICATION.md`.

## Combined project logs

Luna passed the 12-frame headless flow in 17.811 seconds including compilation on 2026-10-12: [result](../../../.verification/headless/20261012T235355Z-67ea0335/result.json). Combined/filtered/recreated project rows, follow/pause, copy feedback, return-to-container routing and Logs/Operations coexistence were inspected. A suspected overlapping row was reassessed: it ends at the scroll viewport boundary, with no pixels over the dock. No concrete visual defect remained. Native clipboard behavior is outside this proof.

The final real two-service fixture passed in 5.38 seconds of test execution: [result](../../../.verification/compose/20261012T235850Z-BOKLV8/result.json) and [cleanup](../../../.verification/compose/20261012T235850Z-BOKLV8/cleanup.log). It proved combined stdout/stderr source identities, fresh log timestamps after both services restarted, and attachment to a force-recreated service's new container ID without reopening. The fixture project `ferrydock-compose-verify-1791849530-532402` was removed. Nine unit tests and Clippy passed; copy formatting and timestamp replay bounds have unit coverage. The existing Docker/container-log/terminal regression passed in 21.96 seconds and its fixture was removed. Desktop project-log interaction remains unverified.

## Compose management slice

Luna passed the final expanded headless flow on 2026-10-12 in 12.649 seconds including compilation, retaining nine screenshots. It inspected association/live/failure frames and reported no visible defects; initial and terminal frames were also inspected on the preceding run. Evidence: [result](../../../.verification/headless/20261012T232843Z-b05f1fc1/result.json). This proves keyboard/pointer UI, request guards and fixture operation rendering, not real Compose execution.

The final real Compose worker fixture passed in 3.11 seconds of test execution. It proved label discovery, saved endpoint-scoped association, ordered override files, an environment file, a profile, Stop/Start/Restart daemon effects, streamed output, ignored ambient overrides, real CLI failure/error output, and rejected invalid associations without overwriting settings. Evidence: [result](../../../.verification/compose/20261012T232834Z-P4ydvM/result.json) and [cleanup](../../../.verification/compose/20261012T232834Z-P4ydvM/cleanup.log). Containers and network for `ferrydock-compose-verify-1791847714-345030` were removed. Settings were isolated under the retained evidence directory. The existing Docker/terminal regression also passed, with its fixture removed. Native Compose clicks remain unverified.

## Earlier Git revision and headless GPU proof

The current application and verifier use GPUI-CE Git revision
`43dacc568497c615c55a1ea0f77ab381c0d9865b`; all 19 GPUI packages in the lockfile
share it. No upstream source is vendored or patched.

```sh
python3 scripts/verify-headless.py
```

Final warm run: `.verification/headless/20261012T205201Z-f9c9f7d1/`.
It passed in **7.333 seconds wall-clock**, with **6.804 seconds** in the
render/input flow. This is one warm measurement on this host, not a guarantee.
Initial compilation, driver setup and competing builds make cold runs slower.

The child had neither `DISPLAY` nor `WAYLAND_DISPLAY`. It created a virtual
1180x820 GPUI view at 2x scale, captured six **2360x1640** WGPU-rendered images,
checked painted glyphs and changed pixels, and exercised real GPUI input:

- Fixture snapshots/log bytes arrive at the Docker worker channel boundary.
- Ctrl+F and typing produce an empty-match view; rendered container-row bounds
  disappear. Restoring the project filter and Escape/Down restores the owned
  row while the standalone row remains absent.
- Pointer clicks at rendered selector bounds pause/resume log follow.
- Clicking Stop produces the correct command at the fixture worker boundary.
- Enter requests a terminal, fixture ANSI bytes paint through the real terminal
  view, and typing `echo verified` plus Enter produces exactly `echo verified\r`
  on the outgoing channel.
- The test context and entity handles tear down without GPUI leak-detector
  failures. The Cargo/test process exits, and all evidence remains.

Read the [result](../../../.verification/headless/20261012T205201Z-f9c9f7d1/result.json),
[flow/timings](../../../.verification/headless/20261012T205201Z-f9c9f7d1/headless.json),
and [input actions](../../../.verification/headless/20261012T205201Z-f9c9f7d1/actions.json).

![Headless GPU terminal](../../../.verification/headless/20261012T205201Z-f9c9f7d1/06-terminal-rendered.png)

The renderer factory explicitly creates `WgpuHeadlessRenderer`; initialization
failure fails the test. It does not use a no-op renderer. The Docker fixture is
in-process channel data: no daemon worker, containers, networks, real shell or
desktop input is involved. These screenshots/assertions are not a golden-image
comparison and do not establish native clipboard/compositor behavior.

## Earlier release regression

Release SHA-256: `94146203b7d106bfcebf177b3ec6cb16175f6e345300dc7cd63e7a0f1b7f7797`.
The native terminal helper passed with the upgraded release in
`.verification/ferrydock/20261012T205704Z-db8e6a18/`. It independently confirmed
the fixture file's unique marker and a **13x128** PTY, captured shell output and
exit, and cleaned up its owned app PID `3530474` and project
`ferrydock-verify-vuvrnuarrmwz`. Seven native screenshots remain.
Read the [native result](../../../.verification/ferrydock/20261012T205704Z-db8e6a18/result.json)
and [cleanup](../../../.verification/ferrydock/20261012T205704Z-db8e6a18/cleanup.json).

The separate release Docker integration check passed real discovery/logs,
stop/start/restart, shell I/O, PTY resize, ANSI parsing, full-screen vi editing,
shell exit and missing Bash. Its dedicated project was removed. Formatting,
all-target/all-feature Clippy with warnings denied, and the five existing unit
tests passed. Headless rendering remains opt-in through `headless-tests`; the
normal application retains its verified X11/XWayland path.

## Archived original native proof

## GPUI-CE migration proof

The terminal flow was re-exercised on 2026-10-12 after migrating to pinned
`gpui-ce` 0.2.2 and `gpui_ce_platform` 0.1.0, with the wgpu Linux renderer and
explicit X11/XWayland startup. Release SHA-256:
`88f14728cae43541125a8736113f9ae291d6be9df9b04435c20552f86d509170`.

Passed run: `.verification/ferrydock/20261012T200150Z-50c7b4cf/`.
Project: `ferrydock-verify-ehxwnsvsunvw`. Owned application PID: `3137712`.
The same mapped keyboard journey confirmed fixture search/selection, shell
open, typed command execution, separate visible output, a newly created file
inside the disposable fixture, PTY dimensions **13 rows × 128 columns**, and
visible shell exit. Cleanup succeeded; an independent check confirmed that
the process, fixture containers and network were absent. Seven PNGs remain.

[Result](../../../.verification/ferrydock/20261012T200150Z-50c7b4cf/result.json),
[side effect](../../../.verification/ferrydock/20261012T200150Z-50c7b4cf/side-effect-after.json),
[transcript](../../../.verification/ferrydock/20261012T200150Z-50c7b4cf/actions.jsonl),
[cleanup](../../../.verification/ferrydock/20261012T200150Z-50c7b4cf/cleanup.json),
and [terminal screenshot](../../../.verification/ferrydock/20261012T200150Z-50c7b4cf/006-04-confirmed-terminal-output.png).

An earlier CE run, `20261012T195846Z-f5650f3c`, found no owned X11 window before
startup explicitly selected X11. Its PID `3115486` and project
`ferrydock-verify-xvczhzvassze` were cleaned up, with no screenshots captured.
This does not establish a native Wayland rendering failure. Other mapped
flows and the coverage limits below have not been promoted to passing UI
checks by this migration. See the project [verification report](../../../VERIFICATION.md)
for the separate Docker integration check and memory comparison.

## Original upstream GPUI proof

Executed on 2026-10-12 against the Linux release binary, X11 display `:0`, and
`unix:///var/run/docker.sock`. Shipping binary SHA-256:
`d9ff8121d85b5004d18e5f068a1f1ca8e7bce2ba2ed1cd2eb0904c16a602f553`.
The application source was unchanged while generating this skill.

## Passed baseline

```sh
python3 .agents/skills/verify-ferrydock/scripts/native.py doctor --display :0
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature terminal --display :0
```

Final run: `.verification/ferrydock/20261012T193957Z-7adc2617/`.
Project: `ferrydock-verify-eassamsexnsu`. Owned application PID: `2999792`.

- Base and instance Doctors passed; exact PID/title and fixture labels matched.
- Ctrl+F isolated the project; Escape/Down selected its probe. Native screenshots
  showed the full name in both list and detail header.
- Return opened the real embedded shell. Targeted window input typed a unique
  `printf … | tee …; stty size | tee …` command and Return.
- A read-only second view observed a newly created fixture file containing
  `FERRYDOCK TERMINAL VERIFIED eassamsexnsu` and PTY dimensions **13 rows × 128 columns**.
- The final harness required two visible marker occurrences: echoed input and
  separate output. The screenshot below shows actual output and dimensions.
- Typing `exit` and Return showed “Shell exited.”
- Cleanup succeeded. A subsequent independent check confirmed the PID was gone,
  project containers/networks were absent, and all **7 screenshots** survived.

![Confirmed native terminal output](../../../.verification/ferrydock/20261012T193957Z-7adc2617/006-04-confirmed-terminal-output.png)

Read the [result](../../../.verification/ferrydock/20261012T193957Z-7adc2617/result.json),
[side effect](../../../.verification/ferrydock/20261012T193957Z-7adc2617/side-effect-after.json),
[input transcript](../../../.verification/ferrydock/20261012T193957Z-7adc2617/actions.jsonl),
and [cleanup](../../../.verification/ferrydock/20261012T193957Z-7adc2617/cleanup.json).
Artifacts are local and ignored by Git; this note remains useful after export or
deletion of the local evidence, but the links then require that evidence copy.

## Attempts and broader coverage

| Run directory under `.verification/ferrydock/` | Result | Coverage and limitation | Cleanup / retained PNGs |
| --- | --- | --- | --- |
| `20261012T193535Z-f4727f7d` | Failed terminal | Correct fixture appeared, but OCR confused random hexadecimal digits; exact identity assertion failed. The helper now uses unambiguous random letters and enlarged OCR input. | Passed / 8 |
| `20261012T193704Z-b9fb0a8d` | Passed terminal | Real command execution, PTY dimensions and shell exit; final screenshot showed separate output. Final helper adds an explicit output-occurrence assertion. | Passed / 6 |
| `20261012T193800Z-f9d41f5e` | Failed full logs flow | Initial marker and heartbeat were visible after Ctrl+1. Native pointer hover calibration failed; **no click was sent**, so follow pause/resume was not verified. | Passed / 12 |
| `20261012T193957Z-7adc2617` | Passed terminal | Final helper, including independent side effect and separate visible output assertions. | Passed / 7 |

All four attempts were checked independently after cleanup: no owned application
PID, no matching fixture containers/networks, and all recorded screenshots retained.

The terminal flow proves one keyboard entry point and basic shell I/O/initial
PTY dimensions/exit. Discovery, project filtering and keyboard selection are
also exercised as preconditions. It does not prove the other terminal entry
points, Bash, clipboard, resize, selection, alternate screens or error paths.

The full `containers`, `logs`, and `controls` flows require mouse calibration.
The observed multi-monitor desktop failed that precondition in the logs attempt;
keep these flows unverified until pointer mapping is demonstrated with visible
hover feedback. Do not disable the guard or substitute direct Docker lifecycle
commands. `inspect` is mapped from source and has not been executed. The map
explicitly records these boundaries for the next maintenance pass.
