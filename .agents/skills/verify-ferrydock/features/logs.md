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

## How to get to it (user POV)

- Select a container; Logs is the default detail tab.
- Click Logs or press Ctrl+1 while focus is outside the terminal.
- Click Following / Follow paused to toggle follow.
- Click Refresh logs; scroll the output area. Click Copy logs to copy retained output.

## Driving it with native.py

Preconditions: headless checks need cached dependencies and a usable Linux WGPU driver; native checks additionally need the skill's native Doctor and its owned fixture/window.

- **Headless development check:** `python3 scripts/verify-headless.py` delivers fixture log bytes through the event channel, captures their GPU-rendered output, and clicks the rendered `follow` selector twice using GPUI pointer events. Assertions verify pause/resume state and the retained paused frame. This proves the view and toggle, not live daemon streaming or viewport motion under a long stream.

Frame 01 also checks split ANSI colour sequences with green first-line pixels and neutral reset text in the second row. Colour checks are scoped to rendered log-row bounds, rather than matching similarly coloured controls. Unit tests check indexed/RGB attributes, split UTF-8, clean copy and basic CR/backspace/erase-in-line updates. ANSI parsing is cached at ingestion; the view renders only visible styled rows. This does not claim full-screen log cursor control or OS clipboard proof.

The native probe below prints an initial marker and heartbeat every two seconds.

- **Open/read:** `python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature logs --display :0` selects the fixture and presses Ctrl+1. Proof: `FERRY_FIXTURE_READY` and `FERRY_HEARTBEAT` appear in native screenshots/OCR.
- **Pause/resume:** the same command clicks Following, waits for Follow paused, then clicks it again. Proof: the corresponding visible follow labels and the click transcript. This checks the toggle state; proving viewport movement under a long stream needs an additional flow.
- **Other entry points:** the Logs tab button, Refresh logs, Copy logs, scrolling and stream failures are source-backed paths. Extend the harness with the actual click/scroll and observable stream/viewport/clipboard change before claiming those paths.

## Gotchas

- Global tab shortcuts are not handled while the embedded terminal owns keyboard focus; control keys then belong to the shell.
- A single heartbeat on screen does not prove multiple live arrivals. The basic flow checks initial/live markers; a timing/count assertion is needed for ongoing streaming claims.
- Logs retain a bounded UTF-8 buffer, so this is not an archive of all container output.
- Mouse steps require successful hover calibration; missing calibration fails without clicking.
