# Combined project logs

Watch all Compose service/container streams from one project workspace.

## Sub-features

- `project-logs.membership`: label-scoped streams, including stopped-container tails and replacement IDs.
- `project-logs.sources`: timestamps and service/container prefixes on combined output.
- `project-logs.filter`: all services or one service without restarting streams.
- `project-logs.follow`: pause/resume scrolling while bounded collection continues.
- `project-logs.copy`: copy the filtered retained history.
- `project-logs.retry`: resume existing IDs with timestamp replay suppression.

## How to get to it (user POV)

- Click a Compose project heading. Logs opens by default; no file association is required.
- Watch source-prefixed lines, choose a service button or All services, and toggle Following/Follow paused.
- Copy logs copies the currently filtered retained output; Refresh logs starts a new tail.
- Operations opens the project's action output. Start/Stop/Restart automatically select Operations; return to Logs to watch services.
- Selecting a container row returns to its individual log stream. Drag the dock divider to adjust the workspace.
- Ctrl+4 opens the selected container's Compose project logs while the main list has focus.

## Driving it

Run `python3 scripts/verify-headless.py`. The flow verifies a project stream request without configuration, combined source rows, filtered rendered bounds, follow/pause and copy feedback, a replacement member's line, and correct routing back to individual logs. Inspect all/filter/recreated project frames. This uses fixture records and proves GPUI input/rendering, not real Docker or native clipboard.

Run `bash scripts/verify-compose.sh` separately. The production worker follows the uniquely owned two-service fixture with stdout/stderr markers. The check verifies both services, logs after project restart and a new container ID after force-recreating the fixture's extra service without reopening the log view. It retains source samples, isolates settings, and confirms scoped cleanup. It does not drive desktop input.

Unit checks prove retry replay counts (including identical same-timestamp lines), late live-line acceptance, global history bounds and service-filtered copy formatting. A passing UI check alone does not establish those transport effects.

The headless flow also fills 1,500 retained records and feeds a 10,000-line batch burst. Check `log-performance.json`, latest-line bounds and frames 13/14: work must stay below 80 rendered rows, follow must reach the tail, and controls must respond. The production Compose check emits 10,000 real records and requires bounded batches with no lost lines. Unit checks separately prove batch order and payload bounds.

For native load, run `cargo build --locked` and `python3 scripts/verify-log-performance.py --display :0` (or select a freshly built release with `--binary target/release/ferrydock`). It owns ten disposable containers and an X11/XWayland window, validates scope, measures CPU and memory, checks search while logs stream, preserves PNGs/OCR/results, and performs scoped cleanup. It takes focus and must not overlap another native-input session. Require all success/cleanup flags in its result; distinguish capture/OCR overhead from application latency.

## Gotchas

- Project membership is scoped by labels on the displayed Docker endpoint. Do not associate files merely to read logs.
- Initial tails are 200 lines per container; total retained history is 1,500 lines. Output is merged as received, not globally sorted by timestamp.
- Follow paused freezes scrolling, not collection. Old records continue to expire at the bound.
- Unsupported Docker logging drivers or stream failures appear as source-specific errors. Running streams retry with a short delay; stopped streams reopen when state changes.
- No native clipboard/Wayland/X11 interaction proof is claimed by these checks. A partial list row at the dock boundary is normal scroll viewport clipping when no pixels overlap the dock.
