# Logs

Read the selected container's output, follow live lines, pause/resume follow, and refresh the log stream.

## Sub-features

- `logs.initial`: show existing stdout/stderr.
- `logs.live`: append new fixture heartbeat lines.
- `logs.follow`: pause/resume following output.
- `logs.refresh`: reconnect the log stream.
- `logs.copy`: copy retained output to the desktop clipboard.
- `logs.scroll`: navigate retained output with the dock scroll area.
- `logs.ansi`: colours/common styles, split escape sequences/UTF-8, readable copied text and basic single-line progress replacement.
- `logs.lifecycle`: timestamped Docker annotations for create/start/stop/restart/exit/signal/OOM/pause/resume/removal, including actions from other clients.
- `logs.lifecycle-details`: plain-language shutdown/force-stop/process-exit messages; hover a marker in container or project logs for raw Docker action, signal or exit code. Exit 137/143 alone is not labelled as an error, OOM or crash.
- `logs.lifecycle-summary`: a confirmed Docker restart condenses its nearby shutdown/exit/stop/start events into one styled annotation, with the complete event sequence on hover. Independent stop/start operations, process errors and OOM events remain visible.
- `logs.lifecycle-stop`: known shutdown/force-stop signals form one stop-request marker; stop and neutral process exit form one stopped confirmation, regardless of event order. A later start stays separate. Raw signals and exit codes stay in hover details; process errors and OOM remain explicit.

## How to get to it (user POV)

- Select a container; Logs is the default detail tab.
- Click Logs or press Ctrl+1 while focus is outside the terminal.
- Click Following / Follow paused to toggle follow.
- Click Refresh logs; scroll the output area. Click Copy logs to copy retained output.

## Driving it with native.py

Preconditions: headless checks need cached dependencies and the GPUI Kit test runtime; native checks additionally need the skill's native Doctor and its owned fixture/window.

- **Headless development check:** `python3 scripts/verify-headless.py` delivers fixture log bytes through the event channel and renders their output without screenshot readback. GPUI pointer events pause/resume follow and drag both vertical and horizontal scrollbars over long retained records. Assertions verify changed scroll offsets, pause/resume state, and return to the tail when following resumes. Named checkpoints cover logs-paused, logs-scrollbar-top, logs-scrollbar-dragged and logs-scrollbar-following. The logs-open-at-bottom frame also verifies that initial history opens at its newest records. This proves rendering and viewport movement with fixture output; real daemon streaming, Refresh logs and the OS clipboard require separate proof.

The containers-and-logs checkpoint also exercises split ANSI sequences and visible rows. Headless tests do not assert glyph colors; inspect those through native verification. Unit tests check indexed/RGB attributes, split UTF-8, clean copy and basic CR/backspace/erase-in-line updates. ANSI parsing is cached at ingestion; the view renders only visible styled rows. This does not claim full-screen log cursor control or OS clipboard proof.

The native probe below prints an initial marker and heartbeat every two seconds.

- **Open/read:** `python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature logs --display :0` selects the fixture and presses Ctrl+1. Proof: `FERRY_FIXTURE_READY` and `FERRY_HEARTBEAT` appear in native screenshots/OCR.
- **Pause/resume:** the same command clicks Following, waits for Follow paused, then clicks it again. Proof: the corresponding visible follow labels and the click transcript. This checks the toggle state; proving viewport movement under a long stream needs an additional flow.
- **Other native entry points:** the Logs tab button, Refresh logs, Copy logs, native scrolling and stream failures are source-backed paths. Extend the native harness with the actual click/scroll and observable stream/viewport/clipboard change before claiming those native paths. Headless scrollbar proof above does not establish native scroll gestures.

## Gotchas

- Run `python3 scripts/verify-headless.py --flow lifecycle` for the focused Kit input/layout flow. It delivers production-shaped worker markers, drives container/project navigation, Refresh logs, Copy logs and service selection, and checks ordering, refresh retention, source isolation, replay deduplication, stale-session rejection, endpoint isolation, gap/recovery status and Light rendering. Named checkpoints cover lifecycle-container, lifecycle-project, lifecycle-events-reconnecting/recovered and lifecycle-light. This is fixture worker/input/render proof; it does not operate real containers or prove the OS clipboard.
- Run `bash scripts/verify-docker.sh docker_external_lifecycle_markers` for production listener proof. A separate Docker CLI stops/starts/restarts only the owned fixture; the test checks the observed actions, Docker state and Compose identity. The Unix-socket unit fixture interrupts `/events`, verifies a second subscription resumes from the cursor without duplicate markers, and cancels the follower. It does not interrupt the shared daemon.
- At most 512 lifecycle markers are cached per current endpoint in memory, independent of the open view; the displayed log keeps its 1,500-row bound. Markers survive log refresh, stream reopen and navigation, and reset when FerryDock closes or changes endpoint. They are FerryDock annotations, not writes to container stdout/stderr. Marker timestamps are explicitly UTC (`Z`).
- The event stream retries every two seconds with bounded replay suppression. Docker retains limited event history, so an interruption leaves a visible possible-gap warning even after recovery. Automatic crash recovery is represented by the actions Docker actually emits (for example exit/start); a new container ID is represented by create/start/removal, without inferring a restart.
- The focused lifecycle GPU flow also reproduces signal 3, signal 9 and exit 137, requires one completed-restart marker, and hovers its full six-event details in both container and project rows. It verifies live project annotation replacement preserves interleaved application records and their ANSI state. Inspect lifecycle-friendly-restart-light, lifecycle-signal-details-light/project-signal-details-light and lifecycle-grouped-project-light/dark/container-dark. Markers use the pinned Kit Tag, a tinted background and accent border; ordinary output retains its original rendering. Copying logs preserves `[Docker]` on the readable summary messages.
- Grouping requires an explicit `restart` event, a matching container ID and nearby stop/exit plus start phases. It examines at most 16 preceding events over five minutes, with at most ten seconds between the final stop/exit and start, and between start and restart confirmation. Raw events remain in the bounded session cache; no timing inference turns a separate stop/start into a restart. A slow or incomplete sequence may retain additional markers.
- Stop grouping examines at most 16 events over five minutes, ending at a later start/restart or unrelated lifecycle action. After completion, only adjacent stop/exit events within ten seconds are merged; a missing request is not invented. Known shutdown and kill signals update the same request while pending. The focused flow requires request/stopped/start counts of one each, hover diagnostics and refresh retention in both views; inspect lifecycle-stop-container-dark/details-dark/then-start-container-dark/project-dark/project-light. The real Docker check now requires exactly two stop markers and one restart marker from the external CLI operations.

- Global tab shortcuts are not handled while the embedded terminal owns keyboard focus; control keys then belong to the shell.
- A single heartbeat on screen does not prove multiple live arrivals. The basic flow checks initial/live markers; a timing/count assertion is needed for ongoing streaming claims.
- Logs retain a bounded UTF-8 buffer, so this is not an archive of all container output.
- Mouse steps require successful hover calibration; missing calibration fails without clicking.
