# Volumes

Entry point: **Workspace → Volumes**. Search names, drivers, or Compose project
labels with the Kit Input; click rows or use Up/Down with list focus. Read driver,
scope, creation date (UTC), Docker-host mountpoint, labels/options and container
references. **Gather volume sizes** explicitly requests volume-only disk usage
from Docker in an independent background job. Values use MiB; unavailable or
unsupported measurements stay last in either sort direction. The button shows
loading state and a UTC completion time. Ordinary inventory refresh, Ctrl+R,
workspace changes and same-endpoint reconnects retain the app-session cache and
never recalculate it. Gathering errors preserve previous measurements. Recreated
volumes or a different endpoint discard old values. Disconnect invalidates an
in-flight gather; late results cannot overwrite the cache.
Click referenced containers to return to their container view, including stopped
containers. Switching workspaces preserves existing container/log/terminal state.

**Age** uses the same relative wording as Images, from minutes through years.
Missing/invalid dates show Unavailable; future dates are shown explicitly. Kit column buttons sort Name, Driver,
Age, Size or References in either direction. Age uses raw creation instants; unavailable
ages stay last. Sort survives search and refresh. Ctrl-click toggles selection;
a normal click selects one. Selected names survive sorting/search/refresh, and
selection counts identify hidden targets.

**Remove volume…** or **Remove selected (N)…** reviews all selected volumes in a
Kit dialog, including hidden and protected targets, explaining permanent data
deletion. Cancel/Escape does not mutate Docker. Pending requests block duplicates
and conflicting Docker mutations. Fresh worker discovery includes stopped
containers; sequential deletion uses force=false, with no prune call, and
continues after each failure. Notifications show removed/failed counts and up to
three failure examples. Successful volumes leave selection; failures stay selected
while they still exist. Disconnect invalidates review and pending state.

**Prune…** opens a Kit dialog with **Anonymous volumes only** and **All unused
volumes**. The first removes unused volumes created without an explicit name;
the second includes named volumes. Both cover the connected daemon regardless of
search or selection and keep running/stopped-container references. The dialog
warns that stored data is permanently deleted; Cancel/Escape sends no command,
and generic Enter does not pick a mode. Accepted pruning survives workspace
changes and blocks conflicting container, image and Compose mutations. Completion
reports deleted volumes and reclaimed MiB, reconciles names/selection/cache, and
refreshes discovery without recalculating surviving sizes. Late operations after
disconnect are ignored. API 1.42+ is required for both modes.

Run `python3 scripts/verify-headless.py --flow volumes` for focused GPUI input and
WGPU rendering proof. The default full application flow includes these checks
as well. Inspect `volumes-*` frames for inventory, youngest/oldest-first age,
aligned columns, multi-selection and hidden-target review, partial/full removal,
stopped reference, no matches, pending/failure/success, Light theme, 10,000-row
virtualization/search, navigation, discovery failure and disconnection. Tests
send real keyboard/pointer input and assert sorted and painted order, unavailable
ages last, Ctrl-click clearing/toggling, selection retention, exact names including
hidden/protected targets, Cancel/Escape, references appearing during review,
stale generations/operation IDs and session preservation. Inspect the gathering,
smallest/largest-first sizes, error-retention, explicit regather and reconnect
frames. Size assertions cover one outgoing request per click, duplicate loading
guards, numeric order (2 MiB before 10 MiB), unavailable last, normal refresh and
inventory-error retention, successful completion while another workspace is open,
explicit regather, recreated identity, same/different endpoints, and stale results
after disconnect. Worker events are fixtures; these tests do not contact Docker.

`cargo test --locked --test volume_sizes` uses owned temporary Unix HTTP sockets
to assert the volume-only query on the negotiated API, both legacy `Volumes` and
modern `VolumeUsage.Items`, zero/missing/negative sizes, empty results, Docker and
malformed-response errors, and the API 1.42 guard. No user's daemon is contacted.

The headless prune flow drives both mode buttons with an empty selection and a
filter, Cancel/Escape/generic Enter, loading duplicates, conflicting container/
image controls, stale operation IDs, success/failure, zero/unavailable reclaim,
survivor size/selection retention, navigation and disconnect. Inspect the
`volumes-prune-*` frames.

`cargo test --locked --test volumes_prune` uses owned Unix API sockets to check
`all=false` versus `all=true`, deleted names and reclaimed bytes, empty/zero/
missing/negative results, malformed/Docker errors and an old-API guard that sends
no request. It never contacts the user's daemon or deletes a Docker resource.

Run `bash scripts/verify-volumes.sh` for production worker/daemon proof. It owns
five uniquely labelled volumes and one networking-disabled Alpine container,
writes a known 3 MiB file in its used volume, requests one volume-only size
calculation, and asserts the measured bytes and volume identity. This read-only
calculation covers all eligible volumes on the shared daemon; mutations and
cleanup stay scoped to the fixtures. Normal discovery must emit no size results.
The fixture also checks inventory/configuration and running/stopped guards, then a mixed batch
with protected/missing/unused targets and a successful two-volume batch after
releasing the container reference. Single deletion and fresh inventory preserve
the other fixture volume. Require test success and `result.json` with
`status: passed` and `cleanup_success: true` under `.verification/volumes/<run>/`.
Cleanup removes only the labelled fixture container/volumes and preserves the
Alpine base image. There are no host bind mounts, published ports, global prune
calls, or force volume removal. Settings/evidence use the isolated run directory.

Native desktop Volumes input, macOS runtime, other viewport sizes, creation,
configuration editing and real daemon-wide prune effects are outside this proof. CI wiring alone is
not evidence of a passing run; record observed results and scoped cleanup.
