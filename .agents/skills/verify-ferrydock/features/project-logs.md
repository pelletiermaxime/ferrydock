# Combined project logs

Watch all Compose service/container streams from one project workspace.

## Sub-features

- `project-logs.membership`: label-scoped streams, including stopped-container tails and replacement IDs.
- `project-logs.sources`: coloured service/container prefixes matching the container list; application dates preserved without Docker timestamp prefixes.
- `project-logs.filter`: all services or one service without restarting streams.
- `project-logs.follow`: pause/resume scrolling while bounded collection continues.
- `project-logs.copy`: copy the filtered retained history.
- `project-logs.retry`: resume existing IDs with timestamp replay suppression.
- `project-logs.ansi`: cached per-container payload colours/styles independent of source-label colours and clean copied text.
- `project-logs.order`: Docker timestamp order across services, including older tails arriving after recent restart output.
- `project-logs.lifecycle`: Docker lifecycle annotations use container/project/service event metadata, participate in ordering/filter/copy and survive refresh/navigation within the bounded session cache.
- Confirmed restarts replace their related annotations with one Docker badge/tinted row. The replacement preserves application records, chronological position and per-container ANSI state. Hover shows the complete underlying event sequence.

## How to get to it (user POV)

- Click a Compose project heading. Logs opens by default; no file association is required.
- Watch source-prefixed lines, choose a service from the searchable picker or All services, and toggle Following/Follow paused.
- Copy logs copies the currently filtered retained output; Refresh logs starts a new tail.
- Operations opens the project's action output. Start/Stop/Restart automatically select Operations; return to Logs to watch services.
- Selecting a container row returns to its individual log stream. Drag the dock divider to adjust the workspace.
- Ctrl+4 opens the selected container's Compose project logs while the main list has focus.

## Driving it

Run `python3 scripts/verify-headless.py`. The flow verifies a project stream request without configuration, combined source rows, filtered rendered bounds, follow/pause and copy feedback, a replacement member's line, and correct routing back to individual logs. Named checkpoints cover all/filter/recreated project states. This uses fixture records and proves GPUI input/rendering, not real Docker or native clipboard.

The flow also clicks Refresh logs, receives recent restart output before 80 older records from another service, and requires that restart to stay visible at the bottom with Following active. Checkpoints cover project-logs-restart-ordered and project-logs-restart-filtered for all-services and service-filtered rendering.

Run `bash scripts/verify-compose.sh` separately. The production worker follows the uniquely owned two-service fixture with stdout/stderr markers. The check verifies both services, logs after project restart and a new container ID after force-recreating the fixture's extra service without reopening the log view. It retains source samples, isolates settings, and confirms scoped cleanup. It does not drive desktop input.

Unit checks prove retry replay counts (including identical same-timestamp lines), late live-line acceptance, timestamp ordering across offsets, stable equal-time records, retention of recent output when older history exceeds the bound, ANSI continuation and service-filtered copy order. A passing UI check alone does not establish those transport effects.

The headless all/filtered/recreated checkpoints assert visible source rows/markers and distinct source-color assignments. Light-mode checkpoints check visibility. Pixel colors and contrast require separate native visual inspection. Unit checks separately verify style continuation per container ID, isolation between replicas of one service, resets, Docker timestamp removal with application dates preserved, clean copy and decoder cleanup after a source disappears and its records expire. The raw transport/replay cursor still consumes original records; ANSI styling is applied in the cached UI history.

The headless flow also fills 1,500 retained records and feeds a 10,000-line batch burst. Check `log-performance.json`, latest-line bounds and the project-logs-busy/project-logs-batched-busy checkpoints: work must stay below 80 rendered rows, follow must reach the tail, and controls must respond. The production Compose check emits 10,000 real records and requires bounded batches with no lost lines. Unit checks separately prove batch order and payload bounds.

For native load, run `cargo build --locked` and `python3 scripts/verify-log-performance.py` (or select a freshly built release with `--binary target/release/ferrydock`). It prefers native Wayland in a Wayland session; `--backend x11 --display :0` explicitly selects XWayland. The demonstrated Wayland adapter requires KDE/KWin and the tools/input access listed in the skill. It owns ten disposable containers, its window, and any temporary input daemon/socket, validates scope, measures CPU and memory, checks search while logs stream, preserves PNGs/OCR/results, and performs scoped cleanup. It takes focus and must not overlap another native-input session. Require all success/cleanup flags in its result, including `input_cleanup`; distinguish backend-specific typing/capture/OCR overhead from application latency.

## Gotchas

- The focused `--flow lifecycle` Kit flow and `docker_external_lifecycle_markers` Docker check are described in [Logs](logs.md). Project markers decode independently of application ANSI state. Event listener state is shown separately in the Kit status bar, including a persistent possible-gap notice after recovery. Container replacements keep their distinct IDs; event metadata still routes removed-container markers to the correct project.

- Project membership is scoped by labels on the displayed Docker endpoint. Do not associate files merely to read logs.
- Initial tails are 200 lines per container; total retained history is the newest 1,500 lines ordered by Docker timestamp. Equal-time records preserve receive order; untimestamped diagnostics follow the newest retained record.
- Follow paused freezes scrolling, not collection. Old records continue to expire at the bound.
- Unsupported Docker logging drivers or stream failures appear as source-specific errors. Running streams retry with a short delay; stopped streams reopen when state changes.
- Headless/worker checks do not prove native desktop or clipboard integration. The separate native performance helper proves owned Wayland or X11/XWayland log reading and search, with `--ansi` for real coloured UTF-8 output; filter/pause clicks and clipboard remain outside that helper. Native Wayland automation currently covers KDE/KWin. A partial list row at the dock boundary is normal scroll viewport clipping when no pixels overlap the dock.
