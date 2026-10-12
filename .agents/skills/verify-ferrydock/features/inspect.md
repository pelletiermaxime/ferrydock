# Inspect

Read Docker inspection JSON for the selected container in its detail dock.

## Sub-features

- `inspect.open`: show inspection details for the current selection.
- `inspect.identity`: confirm the displayed container ID and configuration.
- `inspect.scroll`: scroll through the JSON.
- `inspect.errors`: show daemon inspection failures.

## How to get to it (user POV)

- Select a container row, then click Inspect in the detail dock.
- Press Ctrl+3 while focus is outside the terminal.
- Scroll the inspection output to read additional fields.

## Driving it with native.py

Preconditions: skill Doctor passes; helper owns/proves fixture selection; terminal does not own keyboard focus.

- **Open/identity:** `python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature inspect --display :0` selects the fixture and presses Ctrl+3. Proof: the full fixture ID and Config appear on the native screen, corroborated with owned fixture inspection identity in `inspection-observed.json`.
- **Other entry points:** the Inspect tab button, JSON scrolling and daemon errors are source-backed paths; extend the harness with actual clicks/scroll/error fixtures and corresponding screenshots before marking them verified.

## Gotchas

- Inspection JSON can contain environment values. Keep evidence scoped to the disposable fixture; do not screenshot inspection details of the user's containers.
- Full IDs are long; OCR failure is a verification failure, not permission to bypass the native view.
- A valid daemon response alone does not prove the user can see it.
