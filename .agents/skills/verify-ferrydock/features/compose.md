# Compose projects

Associate project files and manage the project from its bottom-dock workspace.

## Sub-features

- `compose.discovery`: project/service labels and ordered file suggestions.
- `compose.association`: save directory, ordered files, environment files and profiles per endpoint/project.
- `compose.controls`: Start, Stop and Restart with conflict guards.
- `compose.output`: live CLI output and the latest retained result, including failures.
- `compose.persistence`: keep saved associations and containerless projects across launches.

## How to get to it (user POV)

- Click a Compose project heading in Containers. Headings stay available when services are stopped.
- Logs opens by default; Operations shows the most recent action result. Starting an action switches to Operations.
- Choose Associate files, review the suggestions, and enter files/profiles one per line. The directory uses Kit Input and the three lists use Textarea. Tab/Shift+Tab cycle through fields; Ctrl+A selects all, typing replaces the selection, and Ctrl+V pastes. Escape cancels editing.
- Save association. Missing files show a form error; controls remain unavailable until saved.
- Click Start, Stop or Restart. Start creates/starts configured services without building images. Watch output in the bottom dock.
- Switch to a container and back to the heading, then select Operations to inspect the retained result; returning to the project opens Logs. Configuration edits the saved invocation.

## Driving it

Run `python3 scripts/verify-headless.py`. The input-driven flow checks form values at the command boundary, caret/selection editing, Unicode deletion, undo, pointer focus, Tab/Shift+Tab wraparound, disabled editing/cancellation while saving, and Escape cancellation without a save request. It also checks disabled unassociated/conflicting actions, all three operation requests, live output and retained failure. Inspect the association/live/failure frames. This does not execute Compose or write real settings, or prove OS clipboard interaction.

The compose-operation-live frame also renders ANSI-coloured operation output and checks a partial `Starting` line replaced by `Started` through CR/erase-in-line. The operation buffer and viewport retain cached styled records and render only visible rows. This is not full terminal-screen emulation, and the fixture output is not evidence that the Compose CLI emits colours in its current plain-output mode.

Run `bash scripts/verify-compose.sh` separately. It owns a new two-service fixture, pins the local endpoint, and isolates settings. The production worker saves associations and executes Compose. Assertions prove override order, environment file, profile, real lifecycle state/restart time, streamed output, a real CLI failure/error output, ambient override removal, rejected invalid settings and persistence. The script removes its fixture and verifies scoped cleanup. This does not drive desktop input.

Passing both commands proves the headless UI and real process boundaries separately. No native Wayland/X11 Compose interaction, cancellation UI, Rebuild or Down is claimed.

## Gotchas

- Labels are hints; they cannot reconstruct all original environment/profile choices. Review and save before enabling controls.
- Actions apply to all services selected by the saved project configuration and profiles, including replicas.
- Start uses `up --detach --no-build`; it may create/recreate containers and pull missing images. It does not run an image build.
- Paths must be accessible locally. Switching Docker endpoints does not reuse a same-named project's association.
- Closing the application or losing the Docker connection terminates the tracked process but does not roll back Docker changes. Switching between a container and a project does not cancel an ongoing operation.
- Never verify against an existing user project. Settings and daemon state are separate from headless fixture events.
