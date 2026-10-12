# Containers and search

Find containers, see their state and Compose project grouping, and select one to open its detail dock.

## Sub-features

- `containers.discover`: list running containers and group Compose-labelled services by project.
- `containers.search`: filter by visible container/project/image information and show an empty-match state.
- `containers.running`: toggle Running only / All containers.
- `containers.select`: select a row and navigate matching rows with Up/Down.
- `containers.refresh`: refresh discovery manually; daemon events also update the list.

## How to get to it (user POV)

- Launch FerryDock; the main list is the initial view.
- Click Search or press Ctrl+F, then type. Escape/Enter leaves search; Ctrl+A in search clears the filter.
- Click the Running only / All containers button in the header.
- Click a container row or use Up/Down with the main view focused.
- Click Refresh in the header or press Ctrl+R outside terminal/search focus. Scroll the list for additional rows.

## Driving it with native.py

Preconditions: headless checks need cached dependencies and a usable Linux WGPU driver; native checks additionally need the skill's native Doctor and its owned Docker fixture/window.

- **Headless development check:** `python3 scripts/verify-headless.py` supplies snapshots through the Docker event channel, dispatches Ctrl+F/text/Escape/Down through GPUI, and captures initial, empty-search and filtered frames. Assertions check selected fixture identity, zero/nonzero matching rows and more than 1,000 changed pixels after search. This proves rendering/search/selection behavior; the all/running toggle and real daemon discovery are outside this headless flow.

Native commands below record exact keys/clicks in `actions.jsonl`.

- **Discover/select:** `python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature containers --display :0` launches the fixture, uses Ctrl+F and its unique project name, then Escape/Down. Proof: project group and full fixture name in both the list and detail header.
- **No match/restore:** the same command clears the search, types a deliberately absent name, then restores the project filter. Proof: “No containers match your search,” followed by restored list/detail identity.
- **Toggle:** the same command clicks the OCR-located Running only control after hover calibration. Proof: “All containers” appears. A stopped row is exercised separately by `--feature controls`.
- **Other entry points:** row clicks, list scrolling, manual Refresh, standalone grouping and name/image searches are source-backed paths; this flow does not exercise them. Extend the native harness and capture their resulting state before marking them verified.

## Gotchas

- The initial selected container can belong to the user's other projects. Never perform lifecycle actions before filtering and proving the owned fixture selection.
- Search focus consumes Up/Down differently from the main view. Escape restores main-view focus.
- Running only hides stopped rows. Enable All containers before stop/start verification.
- A visible project heading alone does not prove which container the dock controls.
