# FerryDock verification

This is the consolidated record of retained local evidence. Results describe
the source and binary exercised on the stated date; they are not a fresh test
run or proof of every path in the current working tree. For commands and fixture
ownership, see the [development guide](docs/development.md#verification) and
[verification skill](.agents/skills/verify-ferrydock/SKILL.md).

## Latest UI evidence — 13 October 2026

The v0.4.0 GPUI Kit/Fast headless flow passed **54 checks with 37 PNGs**, process exit 0,
and owned-process teardown:
[result](.verification/headless/20261013T104227Z-9a1d0a56/result.json),
[checks and frames](.verification/headless/20261013T104227Z-9a1d0a56/headless.json),
[actions](.verification/headless/20261013T104227Z-9a1d0a56/actions.json).
The evidence directory is pinned with `.keep`.

The run renders the real view at 1180×820 logical / 2360×1640 pixels and drives
GPUI keyboard/pointer input. Coverage includes:

- Discovery/grouping, search editing and restored selection, running-only
  filtering, resource menus, and unavailable/duplicate action guards.
- FerryDock/Dark/Light themes, reset, isolated saved preferences, Light ANSI
  readability, and terminal session/focus preservation across settings.
- Log scrollbars and tail following, container/project/operation ANSI colours,
  service filtering, recreated members, and timestamp preservation across tabs.
- Button focus/activation, tabs, divider dragging, read-only Inspect and search,
  success/error notifications, and terminal input-byte routing.
- Compose association editing, ordered configuration, guarded operation requests,
  live/failure output, and rejection of stale events after disconnect.

The 10,000-record burst retained 1,500 records and rendered 23 rows. Tail following
and controls passed:
[metrics](.verification/headless/20261013T104227Z-9a1d0a56/log-performance.json).
This uses fixture events at the worker boundary. It starts no Docker worker,
executes no real shell/Compose action, and sends no desktop input.

## Worker and regression evidence — 13 October 2026

The v0.4.0 release checks passed formatting, strict all-target/all-feature
Clippy, 21 library tests, seven Python helper regressions, and real Docker and
Compose integration checks. The Python regressions mock process/Docker boundaries
while executing cleanup code; they do not establish real GDB execution.

The Docker fixture passed discovery, lifecycle reconciliation, initial log
history/live resume, interactive shell and PTY resize, ANSI parsing, full-screen
editor transport, shell exit, and missing-Bash error checks. Its owned container
and network were removed after the test.

The retained [Compose result](.verification/compose/20261013T104427Z-gwmGCT/result.json)
and [cleanup log](.verification/compose/20261013T104427Z-gwmGCT/cleanup.log)
record 14 checks against a unique two-service fixture: saved endpoint-scoped
associations, ordered overrides/environment files/profiles, Start/Stop/Restart,
streamed success/failure output, ambient override isolation, rejected invalid
settings, stdout/stderr identity, restart/recreation log recovery, and a
10,000-record burst. The fixture containers and network were removed. This
evidence directory is pinned with `.keep`.

These checks exercise the release source after the UI module refactor and
appearance/menu work. The UI flow establishes input and rendering separately
from the worker tests' real daemon mutations.

## Earlier desktop evidence

These runs predate the GPUI Kit/Fast UI migration and retain their original scope.
They are useful baselines for paths requiring new desktop verification.

| Run | Exercised behavior | Limits |
| --- | --- | --- |
| [X11/XWayland terminal, 12 October](.verification/ferrydock/20261012T205704Z-db8e6a18/result.json) | Fixture discovery/search/selection, real shell input/output, independently observed marker file, initial 13×128 PTY, shell exit, scoped cleanup | Other terminal entry points, Bash, native resize, selection, clipboard, and alternate screens were not exercised |
| [Wayland smoke, 12 October](.verification/wayland/20261012T214414Z-67466d6e/result.json) and [X11 launch](.verification/wayland/20261012T214414Z-67466d6e/x11-result.json) | Visible owned window on both backends; Wayland used an intentionally unavailable socket | No Docker-backed Wayland interaction, typing/resize/clipboard/IME proof |
| [Native ANSI load/search, 13 October](.verification/log-performance/20261013T014129Z-a532d541/result.json) | Owned X11/XWayland window, ten disposable containers producing nominally 500 coloured records/s, visible live logs, search, CPU/RSS sampling, scoped cleanup | No native service-filter/pause/copy controls or Wayland input; measurements are host/build-specific |

The full native logs flow previously stopped at pointer hover calibration on the
multi-monitor display. No click was sent. Native mouse-based container lifecycle
and running-only flows, and the native Inspect helper, remain unverified. A
scripted entry point alone is not evidence that it passed.

## Coverage and retention

Headless assertions establish real GPUI input/rendering at the tested viewport,
without golden-image comparisons. Worker tests establish daemon/CLI behavior
separately. OS clipboard, compositor integration, IME/accessibility, additional
viewports, full editor folding/scroll gestures, and fresh native UI interaction
after the Kit migration need separate proof.

Package installation and AppImage startup/search checks are defined in the
[release workflow](.github/workflows/release.yml). Those workflow definitions
are not a local execution result; packaging was not rerun for the UI migration.

Evidence is local under the ignored `.verification/` directory. Links require
the retained artifacts and may expire under helper retention unless pinned or
exported. Consult the [feature map](.agents/skills/verify-ferrydock/features/README.md)
for commands, entry points, and per-feature coverage boundaries.
