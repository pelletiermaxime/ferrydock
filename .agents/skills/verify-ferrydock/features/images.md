# Images

Entry point: **Workspace → Images**. Search tags, digests, or immutable IDs with
the Kit Input; select Kit rows by click or Up/Down. Read image size in MiB,
age, full ID, local tags and container references; the detail panel retains the
exact creation date in UTC. Referenced
containers navigate back to the Containers workspace, including stopped rows.
The bottom dock uses Kit Overview, History and Inspect tabs. History reads
Docker's newest-first commands in a compact Kit DataTable with resizable headers
in #/Command/Size (MiB)/Created (UTC)/Layer ID order. Ctrl+F searches commands;
Home/End select the first/last row. The Kit Columns menu toggles metadata fields,
all visible by default. Choices and resized widths survive image selection.
Missing values show —. Right-click for Copy command, View command or Copy layer ID;
Ctrl+C copies a selected command. Double-click or Enter opens the full instruction
in a read-only searchable/selectable Kit Editor. Inspect reuses the read-only Kit JSON Editor
with syntax colours, selection, folding and built-in Ctrl+F search. Ctrl+1/2/3
select the image tabs. Detail reads are lazy, independently report errors, and
retry on Refresh. Only the selected image's results are retained.
Click the Image / ID, Age, Size (MiB), or References header button to sort ascending;
click again for descending. The active arrow indicates direction. Unavailable
sizes/dates remain last, and sorting survives search and refresh with selection intact.
Ctrl-click toggles membership in the image selection; a normal click selects
one image. Selection counts identify hidden selections, and the Kit confirmation
popup includes all selected IDs. Cancel/Escape closes it without mutation.
Batches continue after refusals and show notification counts/failure examples;
failed images stay selected if they still exist. Selected-image removal uses no
force or parent pruning.

**Prune…** opens a Kit dialog with Cancel, All untagged images, and All unused
images. Pruning covers the connected daemon regardless of search/selection and
preserves images referenced by running or stopped containers. Pending pruning
blocks conflicting Docker actions; completion refreshes inventory and reports
reclaimed MiB or unavailable space.

Use `python3 scripts/verify-headless.py` for GPUI input and GPU rendering proof.
Inspect the `images-*` screenshots: inventory, stopped reference, search/no
matches, untagged/unavailable size, removal review, removing/failure/success,
Light, large inventory/search, discovery failure and disconnection. The flow
asserts rendered column alignment, fewer than 40 rendered rows for 10,000 images,
preserved container/project session state, immutable-ID removal commands,
Cancel and duplicate-operation protection, references appearing during review,
stale snapshots, and disconnected operation results. Discovery is supplied at
the worker boundary; these checks perform no real Docker mutation.
The detail flow clicks all tabs and uses the keyboard shortcuts, verifies
read-only JSON typing and search, syntax-coloured glyphs, history context-copy feedback,
empty/error/loading states, refresh retry, stale generation/wrong-ID/closed
workspace/disconnect guards, reconnect requests and 2,000-entry virtualization.
Inspect `images-history*` and `images-inspect*` screenshots, including the Light
history frame. Table checks cover header/cell alignment with long commands, a
53-command sample, actual divider dragging, column toggles retaining width and
row selection, filtered original step numbers, pointer/keyboard command copying,
full-command dialog read-only typing/search and double-click/Enter/Close,
horizontal wheel scrolling, 2,000-row virtualization and Home/End navigation. Native clipboard and
folding remain outside this flow.
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
disconnected invalidation. Age checks sort raw timestamps both ways (youngest
first ascending, oldest first descending),
keep unavailable values last, and assert aligned date cells. Inspect the
`images-age-oldest-first` and `images-age-youngest-first` frames.
Inspect the `images-multiple-*` screenshots, including hidden selection/review
and completion feedback. No removal-results panel should appear in the workspace.

Prune checks drive Cancel/Escape, neutral Enter, both mode buttons, pending and
duplicate guards, stale completion, success/error/unavailable-space feedback,
and refresh with selection preserved. Inspect `images-prune-review`,
`images-prune-pending`, `images-prune-success` and `images-prune-feedback` frames.
`cargo test --locked --test images_prune` verifies the real Bollard request filters,
reclaimed bytes and daemon-error propagation against an owned Unix socket API
fixture. It performs no real Docker pruning.

Use `bash scripts/verify-images.sh` for production worker/Docker proof. It builds
four uniquely tagged/labelled Alpine-based images, starts one container with
networking disabled, checks running and stopped references, rejects referenced
image removal, reads production image history commands and inspection JSON
identity/configuration through the worker, surfaces a multi-tag Docker conflict
without force, removes the
unreferenced images and reconciles discovery. A mixed batch keeps stopped-container
and multi-tagged images while deleting an unused image, then a two-image batch
succeeds after removing the owned container. It verifies the other
image and Alpine base remain, then cleans up only its own container and tags.
There are no host mounts, published ports, persistent volumes, or prune calls.
Build cache and the Alpine base remain. Require the integration test to pass,
`result.json` status `passed` and `cleanup_success: true` under the printed
`.verification/images/<run>/` directory. Launch/test/cleanup logs remain there.

Native desktop Images clicks, OS clipboard, macOS runtime, additional viewport
sizes, pulling/tagging and real daemon pruning effects are outside this proof. CI wiring alone
does not establish that a run passed; record observed evidence in the change.
