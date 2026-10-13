# GPUI Kit integration

FerryDock 0.3.0 uses GPUI Kit for its editable fields, buttons, tab bars, workspace divider, Inspect editor, notifications, and container log scrollbars.

| Component | Current behavior |
| --- | --- |
| Input / Textarea | Search and the Compose directory use Input; Compose files, environment files and profiles use Textarea. Editing supports caret movement, selection replacement, Unicode deletion and undo. |
| Button | Toolbar and action controls use real Kit buttons with keyboard focus, disabled behavior, useful tooltips, and loading indicators for container actions, Compose actions and saving. |
| TabBar | Logs / Terminal / Inspect use an underline TabBar; project Logs / Operations use a segmented TabBar. Existing shortcuts and routing remain available. |
| Resizable | A single vertical panel group separates the list from the detail dock. It owns pointer dragging and size constraints, replacing the duplicated handles and root mouse handlers. Opening a terminal or configuration form still expands the dock. |
| Editor | Inspect is a read-only JSON editor with syntax highlighting, line numbers, folding, selection, copying and built-in search. |
| Notification | Container/Compose completion and copy feedback use in-app notifications. Success feedback auto-hides; errors do not auto-hide. New feedback replaces the previous feedback slot, as with the former banner. |

## Try the integration

```sh
cd /home/maximep/projects/ferrydock
cargo run --locked --offline
```

The updated development binary is built at `target/debug/ferrydock`. An application window opened before this build needs a restart to load the new controls. The app connects to the normal Docker endpoint.

Use Tab to focus a button, then Enter or Space to activate it. Toolbar pointer clicks preserve text focus. The list's Enter and arrow shortcuts run only when the list owns focus, so they cannot steal button activation.

Click a container's Inspect tab or use Ctrl+3. Ctrl+F searches its JSON, Enter moves to the next match, and Escape closes search. Typing cannot change the inspection result. Drag the boundary above the detail panel to resize it; the dock retains a minimum height.

The Compose form preserves ordered files, environment files and profiles. Tab/Shift+Tab cycle through its four fields, Ctrl+A selects existing text, and typing replaces the selection. Enter adds a line in the list fields. Saving disables editing and cancellation; Escape cancels an editable form without saving.

## Implementation

Kit is pinned to `0.7.3` with `gpui-fast` enabled; the direct GPUI engine, platform and WGPU dependencies use matching Fast version `0.1.7`. The development optimization setting also targets `gpui-fast`. Startup initializes Kit and its assets, wraps the app in Root, and applies FerryDock's dark colours. Alacritty continues to handle terminal emulation, and the existing virtualized streamed-log views remain in use.

The engine switched from standard `gpui-pre 0.3.9` to Fast on 13 October 2026. Kit's feature selects Fast across its component layers; direct engine consumers must use the same backend to keep GPUI types compatible. Fast uses `windows-manifest-provided-by-host`, since Kit's upstream build dependency supplies the Windows manifest. Upstream packages remain in the resolved/build graph by Kit's design, but the application uses Fast's public types. The switch preserves the existing lockfile's 842 package versions.

The JSON editor enables Kit's `tree-sitter` feature. In this version that feature includes the JSON grammar; the full language bundle is not enabled. It adds four locked packages, bringing the lockfile from 838 packages in the initial Kit integration to 842. The original CE baseline had 719. These are resolved packages across targets/features, not a count of crates compiled for one platform.

The backend event loop now runs in the window context so it can update the editor and present notifications directly. Existing generation checks, operation guards, Compose configuration handling and retained failure output remain in place. Notifications explicitly use in-app delivery.

The custom text editor, button primitive, tab strips, resize tracking and feedback banner have been replaced by Kit controls. Across `app.rs`, `main.rs`, `ansi.rs` and `terminal.rs`, the complete integration has a raw net increase of 16 lines against the original inherited baseline. The initial search-only trial was +47; adding Compose inputs brought it to +25; the five-component follow-up removes nine more production lines. These counts describe source size rather than behavioral complexity.

## Verification

The final GPU run is
[20261013T050220Z-56b4a43f](../.verification/headless/20261013T050220Z-56b4a43f/result.json).
It passed on `gpui-fast 0.1.7`, exited its owned process, and retained twenty-three 2360×1640 PNGs for a 1180×820 logical viewport. A `.keep` file pins the evidence.

The flow records forty checks, including:

- Existing search editing, container/project ANSI glyph colours, terminal rendering/input routing, ordered Compose configuration, blocked/conflicting controls, operation requests, retained failure output, service filters and recreated members.
- Button focus through Tab, exactly one Refresh through Enter/Space, prevention of list-shortcut leakage, and protection against repeating a loading action.
- Real pointer selection and selected-state observation for both TabBars.
- Real divider dragging through measured pane bounds, minimum dock height, and restoration of the original size.
- Log scrollbar rendering, pointer dragging and resumed tail following.
- Read-only Inspect typing protection, JSON-coloured glyph pixels inside the actual editor text bounds, and a typed search match with replacement unavailable.
- Success notification rendering, its hover-revealed close control, and error persistence beyond the normal success timeout followed by manual dismissal.
- The bounded 1,500-line history and 10,000-record burst, fewer than 80 rendered log rows, tail following and responsive controls after the burst.

The harness commits a hover frame before pressing hover-dependent controls. Motion snapshots settle both real painted-frame time and the fixture executor's clock. These waits verify visible interactions; their duration is not an application performance measurement.

Formatting, strict Clippy, all sixteen library tests and the normal desktop build pass. Fast validation logs are retained under `.verification/gpui-kit-trial/fast-*.log`; the earlier standard-backend component logs remain under `components-*.log`.

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked --offline -- -D warnings
cargo test --lib --locked --offline
python3 scripts/verify-headless.py
cargo build --locked --offline
```

The GPU flow uses real GPUI input and WGPU rendering with fixture events at the worker boundary. It starts no Docker worker, creates no containers, and sends no desktop input. Native Wayland/X11 interaction, OS clipboard/IME/accessibility, full editor folding/scroll gestures, release packaging, and real Docker/Compose execution have not been reverified for this UI migration.

## Integration history and review

The original inherited baseline tree is
`b4de116a64ea32fa476142b0a23644a6f792dc1b`. On 13 October 2026, the isolated Kit delta was applied to `main` through a three-way merge, preserving its newer release metadata and ANSI coverage. Referenced evidence was copied into the main checkout, and the trial worktree was archived with a recoverable snapshot and removed.

Version 0.3.0 includes this integration and the container log improvements.
Container logs load their initial history as one batch and paint the newest
records in the first frame, then resume live output without replaying the
snapshot. Switching containers resets the previous scroll position.

The benchmark source copies, targets, scripts, logs and comparison report were removed as requested. They were not recreated for this component migration. No runtime or binary-size improvement is claimed from these UI checks.

## Screenshots

![Read-only JSON Inspect editor](../.verification/headless/20261013T050220Z-56b4a43f/10-kit-inspect-json.png)

![In-app completion notification](../.verification/headless/20261013T050220Z-56b4a43f/13-kit-action-notification.png)

![Compose Input and Textareas](../.verification/headless/20261013T050220Z-56b4a43f/19-compose-association.png)

## Upstream references

- [Kit integration](https://gpui-kit.com/docs/getting-started)
- [Input](https://gpui-kit.com/component/input) and [Textarea](https://gpui-kit.com/component/textarea)
- [Button](https://gpui-kit.com/component/button), [Tabs](https://gpui-kit.com/component/tabs) and [Resizable](https://gpui-kit.com/component/resizable)
- [Editor](https://gpui-kit.com/component/editor) and [Notification](https://gpui-kit.com/component/notification)
