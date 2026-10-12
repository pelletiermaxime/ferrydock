# FerryDock verification map

This is an incremental map of the current Linux application. It covers the implemented main list and detail dock. Compose project lifecycle/rebuild, networks, volumes, and cleanup pages are planned and have no current user entry point.

Default development command: `python3 scripts/verify-headless.py`. It renders the real view through WGPU without a desktop window, drives GPUI input events, and checks commands at a fixture Docker boundary. It needs a usable Linux graphics driver and cached dependencies, but no Docker daemon. The flow retains screenshots and structured evidence under `.verification/headless/`; see the skill's Doctor and [../PROOF.md](../PROOF.md).

Native runs use the real release binary, an accessible X11 display, and a newly owned Docker fixture/project. Native screenshots/OCR and read-only daemon observations provide a separate desktop/transport proof.

Headless coverage: container/project rendering, keyboard search/empty matches/restored selection, pointer log-follow toggle, outgoing Stop request, Enter to request a terminal, ANSI fixture output, and keyboard-to-terminal byte routing. It does not prove actual Docker operations, real shell execution, clipboard/compositor behavior, golden-image equivalence, or the other mapped entry points.

| Feature | Automated flow | Other entry points and coverage boundary |
| --- | --- | --- |
| [Containers and search](containers.md) | Project isolation, selection, empty search, restore filter, all/running toggle | Row clicks, scrolling, manual Refresh and alternate disconnected states require separate proof. |
| [Logs](logs.md) | Initial/live fixture output, pause/resume follow | Logs tab button, refresh, scrolling and unrelated stream errors require separate proof. |
| [Embedded terminal](terminal.md) | Enter opens shell; type command; corroborate file/PTY; exit | Tab/Open shell buttons, Bash, resize, selection, copy/paste, alternate screen, wheel and error paths require separate native proof. |
| [Container controls](controls.md) | Stop/start/restart through visible controls and daemon observation | Each lifecycle action is scoped to one disposable selected container. |
| [Inspect](inspect.md) | Inspect shortcut, full fixture ID and Config on screen | Inspect tab button and scrolling require separate proof. |

The source-backed user entry points are listed in each file. The embedded-terminal journey has been exercised, including fixture discovery/search/selection. Logs were observed, but the full follow-toggle flow stopped at mouse calibration; lifecycle and all/running mouse steps remain unverified on this display. Inspect has not been run. See [../PROOF.md](../PROOF.md) for execution status and retained evidence. This map is not a complete inventory of all behaviors, daemon failures, terminal escape sequences, or supported platforms.

Each feature file includes the headless command where applicable and its native flow. Keep those proof boundaries distinct. Native `run` owns launch, doctor, selection, drive, evidence and cleanup together. Only claim steps demonstrated by the chosen harness and its observed effects. Add new paths as real features arrive; `/maintain-verification-skill` maintains this map.
