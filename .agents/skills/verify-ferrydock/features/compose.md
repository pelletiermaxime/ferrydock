# Compose projects

Find a project's Compose source and manage it from its bottom-dock workspace.

## Sub-features

- `compose.discovery`: project/service labels and ordered file suggestions.
- `compose.source`: labeled Compose paths, ordered-file tooltip and verified linked Git worktree tag; accessible labeled files associate automatically, saved settings take precedence.
- `compose.association`: save directory, ordered files, environment files and profiles per endpoint/project.
- `compose.controls`: Start, Stop, Restart and Rebuild & restart with conflict guards.
- `compose.cleanup`: reviewed project teardown, optional volume/generated-image removal, stale review guards and association removal after success.
- `compose.output`: live CLI output and the latest retained result, including failures.
- `compose.persistence`: keep saved associations and containerless projects across launches.

## How to get to it (user POV)

- Click a Compose project heading in Containers. Headings stay available when services are stopped.
- Read its Compose path and hover for every file in order and the linked worktree root. Automatic association requires locally accessible directory/files/environment files; use Configuration for profiles or explicit overrides, Associate files when discovery cannot validate the paths.
- Logs opens by default; Operations shows the most recent action result. Starting an action switches to Operations.
- Choose Associate files, review the suggestions, and enter files/profiles one per line. The directory uses Kit Input and the three lists use Textarea. Tab/Shift+Tab cycle through fields; Ctrl+A selects all, typing replaces the selection, and Ctrl+V pastes. Escape cancels editing.
- Save association. Missing files show a form error; controls remain unavailable until saved.
- Click Start, Stop, Restart or Rebuild & restart. Start creates/starts configured services without building images. Rebuild & restart builds images, then recreates and starts configured services. Watch output in Operations.
- Stop retains containers and data. Running only hides the stopped rows, while the project remains available. More → Stop & cleanup opens a Kit Dialog listing project containers, with checked volume-data and generated-image deletion options; Cancel/Escape keeps resources. Confirm starts teardown in Operations. External resources, bind mounts and explicitly tagged images remain. Successful cleanup removes the association and empty heading.
- Switch to a container and back to the heading, then select Operations to inspect the retained result; returning to the project opens Logs. Configuration edits the saved invocation.

## Driving it

The headless flow also renders a linked-worktree source row and hovers its Kit
tooltip. Inspect `compose-source-worktree` and `compose-source-tooltip` for the
primary path, ordered overrides and worktree root. This uses fixture metadata;
local Git discovery and automatic association are checked separately below.

The Compose helper puts its fixture in an owned linked Git checkout. It checks
the production worker's worktree root and automatic ordered-file/environment
association, then restarts a container without saving settings and corroborates
its changed start time. Explicit association tests follow with saved profiles.

The headless flow opens cleanup through the project menu, drives Cancel, Escape
and both checkboxes, checks outgoing cleanup options, rejects changed
configuration/membership, renders failure output and applies successful removal
discovery. Inspect the `compose-cleanup-*` frames. The real Compose helper checks
keep/remove options, profiled and orphan containers, project networks, named and
anonymous volumes, a generated build image, association removal and disappearance.
Owned external volume/network probes, tagged images and a bind marker survive the
product operation; the helper then cleans up those owned probes separately.

Run `python3 scripts/verify-headless.py`. The input-driven flow checks form values at the command boundary, caret/selection editing, Unicode deletion, undo, pointer focus, Tab/Shift+Tab wraparound, disabled editing/cancellation while saving, and Escape cancellation without a save request. It also checks disabled unassociated/conflicting actions, all four operation requests, rebuild progress, duplicate/conflicting action protection, and build failure/output retained after leaving and returning to the project. Inspect the association, operation live/failure, and rebuild live/failure frames. This does not execute Compose or write real settings, or prove OS clipboard interaction.

The compose-operation-live frame also renders ANSI-coloured operation output and checks a partial `Starting` line replaced by `Started` through CR/erase-in-line. The operation buffer and viewport retain cached styled records and render only visible rows. This is not full terminal-screen emulation, and the fixture output is not evidence that the Compose CLI emits colours in its current plain-output mode.

Run `bash scripts/verify-compose.sh` separately. It owns a new two-service fixture, pins the local endpoint, and isolates settings and build inputs. The production worker saves associations and executes Compose. Assertions prove override order, environment file, profile, real lifecycle state/restart time, changed build inputs producing a new image/container with updated contents, recreation of the profiled image-only service, log reattachment after rebuild, failed build output with the running services preserved, a real CLI failure/error output, ambient override removal, rejected invalid settings and persistence. The script removes its containers, network and uniquely labelled built images, including superseded images, and verifies scoped cleanup. Build cache and the base image remain. This does not drive desktop input.

Passing both commands proves the headless UI and real process boundaries separately. No native Wayland/X11 Compose interaction, cancellation UI or Down is claimed.

## Gotchas

- Labels cannot reconstruct every original environment/profile choice. Accessible labeled files enable controls automatically with default profiles; use Configuration to add profiles or environment overrides.
- Actions apply to all services selected by the detected or saved project configuration and profiles, including replicas.
- Start uses `up --detach --no-build`; it may create/recreate containers and pull missing images. It does not run an image build.
- Rebuild & restart uses `up --detach --build --force-recreate`, reuses Docker's build cache and recreates all configured services even when their images are unchanged. A build failure leaves the existing services running; a later startup failure may leave partial Docker changes, which are not rolled back. The selected project's shell session is closed before the action.
- Paths must be accessible locally. Switching Docker endpoints does not reuse a same-named project's association.
- Closing the application or losing the Docker connection terminates the tracked process but does not roll back Docker changes. Switching between a container and a project does not cancel an ongoing operation.
- Never verify against an existing user project. Settings and daemon state are separate from headless fixture events.
