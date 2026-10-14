# Images

Entry point: **Workspace → Images**. Search tags, digests, or immutable IDs with
the Kit Input; select Kit rows by click or Up/Down. Read image size in MiB,
creation date in UTC, full ID, local tags and container references. Referenced
containers navigate back to the Containers workspace, including stopped rows.
Click the Image / ID, Created (UTC), Size (MiB), or References header button to sort ascending;
click again for descending. The active arrow indicates direction. Unavailable
sizes/dates remain last, and sorting survives search and refresh with selection intact.
Ctrl-click toggles membership in the image selection; a normal click selects
one image. Selection counts identify hidden selections, and the Kit confirmation
popup includes all selected IDs. Cancel/Escape closes it without mutation.
Batches continue after refusals and show notification counts/failure examples;
failed images stay selected if they still exist. No force/prune is used.

Use `python3 scripts/verify-headless.py` for GPUI input and GPU rendering proof.
Inspect the `images-*` screenshots: inventory, stopped reference, search/no
matches, untagged/unavailable size, removal review, removing/failure/success,
Light, large inventory/search, discovery failure and disconnection. The flow
asserts rendered column alignment, fewer than 40 rendered rows for 10,000 images,
preserved container/project session state, immutable-ID removal commands,
Cancel and duplicate-operation protection, references appearing during review,
stale snapshots, and disconnected operation results. Discovery is supplied at
the worker boundary; these checks perform no real Docker mutation.
Sorting checks click every header in both directions, verify numeric size/count
ordering and rendered row positions, keep unavailable sizes last, preserve the
selected ID through sorting/filtering/refreshed snapshots, and send no worker
commands. Inspect the `images-size-ascending`, `images-size-descending`,
`images-references-descending`, and `images-name-descending` screenshots.
Multi-selection checks dispatch real Ctrl-modified pointer events, verify Kit
row highlights and empty-selection protection, retain selection through sorting,
filtering and refreshed snapshots, review hidden targets, confirm exact bulk
IDs, reject duplicates/stale results, and check partial/full success selection.
Popup checks include Cancel/Escape, pointer confirmation, stale-reference and
disconnected invalidation. Creation-date checks sort raw timestamps both ways,
keep unavailable values last, and assert aligned date cells. Inspect the
`images-created-oldest-first` and `images-created-newest-first` frames.
Inspect the `images-multiple-*` screenshots, including hidden selection/review
and completion feedback. No removal-results panel should appear in the workspace.

Use `bash scripts/verify-images.sh` for production worker/Docker proof. It builds
four uniquely tagged/labelled Alpine-based images, starts one container with
networking disabled, checks running and stopped references, rejects referenced
image removal, surfaces a multi-tag Docker conflict without force, removes the
unreferenced images and reconciles discovery. A mixed batch keeps stopped-container
and multi-tagged images while deleting an unused image, then a two-image batch
succeeds after removing the owned container. It verifies the other
image and Alpine base remain, then cleans up only its own container and tags.
There are no host mounts, published ports, persistent volumes, or prune calls.
Build cache and the Alpine base remain. Require the integration test to pass,
`result.json` status `passed` and `cleanup_success: true` under the printed
`.verification/images/<run>/` directory. Launch/test/cleanup logs remain there.

Native desktop Images clicks, OS clipboard, macOS runtime, additional viewport
sizes, pulling/tagging and daemon-wide pruning are outside this proof. CI wiring alone
does not establish that a run passed; record observed evidence in the change.
