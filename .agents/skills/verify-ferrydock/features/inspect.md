# Inspect

Read Docker inspection JSON for the selected container in its detail dock.

## Sub-features

- `inspect.open`: show inspection details for the current selection.
- `inspect.identity`: confirm the displayed container ID and configuration.
- `inspect.scroll`: scroll through the JSON.
- `inspect.errors`: show daemon inspection failures.
- `inspect.search`: find matches in the read-only JSON editor without replacement.
- `inspect.presentation`: JSON syntax colors, line numbers and selectable text.

## How to get to it (user POV)

- Select a container row, then click Inspect in the detail dock.
- Press Ctrl+3 while focus is outside the terminal.
- Scroll the inspection output to read additional fields.
- Press Ctrl+F while Inspect owns focus to search the JSON; Enter advances matches
  and Escape closes the search panel. Typing does not edit the inspection result.

## Headless GPU proof

`python3 scripts/verify-headless.py` clicks the real Kit Inspect tab, delivers
fixture-only JSON through the worker event boundary, and checks the editor value.
It verifies nonempty editor text bounds, rejects typed
replacement after Select All, and uses Ctrl+F plus real typing to find a match
with replacement unavailable. The `kit-inspect-json` and
`kit-inspect-search` checkpoints render frames without pixel readback. The flow also drags the real Kit divider to
resize the dock and checks its minimum height. This proves GPUI input/rendering,
not actual Docker inspection, OS clipboard, or every folding/scroll gesture.

## Driving it with native.py

Preconditions: skill Doctor passes; helper owns/proves fixture selection; terminal does not own keyboard focus.

- **Open/identity:** `python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature inspect --display :0` selects the fixture and presses Ctrl+3. Proof: the full fixture ID and Config appear on the native screen, corroborated with owned fixture inspection identity in `inspection-observed.json`.
- **Other native entry points:** the Inspect tab button, JSON scrolling and daemon errors remain outside the native proof. The tab and read-only search have separate headless Kit input/layout proof above.

## Gotchas

- Inspection JSON can contain environment values. Keep evidence scoped to the disposable fixture; do not screenshot inspection details of the user's containers.
- Full IDs are long; OCR failure is a verification failure, not permission to bypass the native view.
- A valid daemon response alone does not prove the user can see it.
