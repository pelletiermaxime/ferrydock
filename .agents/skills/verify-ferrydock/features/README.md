# FerryDock verification map

This is an incremental map of the current Linux application. It covers the main list, container detail dock, and Compose project configuration/Start/Stop/Restart workspace. Rebuild, Down, networks, volumes, and cleanup pages remain planned.

Default development command: `python3 scripts/verify-headless.py`. It renders the real view through WGPU without a desktop window, drives GPUI input events, and checks commands at a fixture Docker boundary. It needs a usable Linux graphics driver and cached dependencies, but no Docker daemon. The flow retains screenshots and structured evidence under `.verification/headless/`; see the skill's Doctor and [the verification summary](../../../../VERIFICATION.md).

Native runs use the real release binary, an accessible X11 display, and a newly owned Docker fixture/project. Native screenshots/OCR and read-only daemon observations provide a separate desktop/transport proof.

Headless coverage includes the UI paths listed below, with fixture events at the worker boundary. It does not prove actual Docker operations, real shell execution, clipboard/compositor behavior, or golden-image equivalence.

| Feature | Automated flow | Other entry points and coverage boundary |
| --- | --- | --- |
| [Appearance and resource menus](appearance.md) | Headless theme switching/reset/persistence, Light logs and terminal, session/focus preservation, menus and unavailable actions | Native settings/menu interactions, OS clipboard, and responsive layouts outside the tested 1180x820 view require separate proof. |
| [Containers and search](containers.md) | Project isolation, selection, empty search, restore filter, all/running toggle | Row clicks, scrolling, manual Refresh and alternate disconnected states require separate proof. |
| [Logs](logs.md) | Headless initial/live output, follow/pause, tabs and scrollbar dragging | Native refresh, scroll gestures and stream errors require separate proof. |
| [Embedded terminal](terminal.md) | Enter opens shell; type command; corroborate file/PTY; exit | Tab/Open shell buttons, Bash, resize, selection, copy/paste, alternate screen, wheel and error paths require separate native proof. |
| [Container controls](controls.md) | Headless Stop request and pending/duplicate guards; real lifecycle effects in Docker integration | Native Start/Stop/Restart helper exists but its mouse flow remains unverified. |
| [Inspect](inspect.md) | Headless Kit tab click, JSON glyph colors, read-only typing guard and search; native helper has a shortcut/identity flow | Native Inspect execution, OS clipboard, folding and full scroll gestures require separate proof. |
| [Compose projects](compose.md) | Headless association form, blocked/unblocked controls, Start/Stop/Restart requests, live/failure output; separate real worker/CLI fixture | Native desktop Compose clicks, Rebuild, Down, and cancellation UI are outside this proof. |
| [Project logs](project-logs.md) | Headless combined/service-filter rows, follow/pause, copy feedback, recreated member, routing and busy-history/burst checks; real multi-service stdout/stderr, restart/recreation and 10k burst; native X11/XWayland log-load/search measurement | Native clipboard/Wayland input, native service-filter/pause clicks, strict timestamp ordering and unbounded historical logs are outside this proof. |

The source-backed user entry points are listed in each file. The embedded-terminal journey has been exercised, including fixture discovery/search/selection. Logs were observed, but the full native follow-toggle flow stopped at mouse calibration; lifecycle and all/running native mouse steps remain unverified on this display. Inspect has headless GPU proof, but its native helper has not been run. See [the verification summary](../../../../VERIFICATION.md) for execution status and retained evidence. This map is not a complete inventory of all behaviors, daemon failures, terminal escape sequences, or supported platforms.

The headless flow also proves Kit Button focus through Tab, Enter/Space activation
without list-shortcut leakage, disabled/loading action protection, both TabBars,
Resizable pointer dragging and minimum height, and visible notification dismissal.
Error notifications remain beyond the normal success timeout. These are virtual
window interactions and fixture-channel requests; no real container action runs.

Each feature file includes the headless command where applicable and its native flow. Keep those proof boundaries distinct. Native `run` owns launch, doctor, selection, drive, evidence and cleanup together. Only claim steps demonstrated by the chosen harness and its observed effects. Add new paths as real features arrive; `/maintain-verification-skill` maintains this map.
