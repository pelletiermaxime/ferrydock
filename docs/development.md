# Developing FerryDock

FerryDock is a Cargo workspace with a desktop package and a small offline
`crates/image-analysis-core` library. The desktop has separate UI, model, Docker worker, and
terminal modules. Docker I/O runs on a two-thread Tokio runtime away from the
GPUI thread. Event delivery is bounded, view updates are batched, and streams
are cancelled when selection changes or the window closes.

For the user guide, see the [README](../README.md).

## Application structure

`src/app.rs` composes the GPUI application and wires its worker channel. Feature
state, controls, and rendering belong to the modules under `src/app/`:

| Module | Responsibility |
| --- | --- |
| `containers.rs` | Discovery/search state, selection, container actions, and the container detail panel |
| `container_list.rs` | Virtualized resource groups and Kit ListItem rows, tags, and empty states |
| `metrics.rs` | Per-container resource history, freshness, compact rows and CPU/memory summary charts |
| `images.rs` | Images workspace, search/selection, virtualized image and reference lists, removal review and operation feedback; Docker discovery/removal remains in `src/images.rs` |
| `image_analysis.rs` | Analysis request state, layer selection, Kit file tree and Findings; export/scheduling remains in `src/image_analysis.rs`, offline accounting in `crates/image-analysis-core` |
| `networks.rs` | Read-only network inventory, configured endpoints, aliases/DNS, virtualized lists and container routing; cancellable discovery/details remain in `src/networks.rs` |
| `volumes.rs` | Volumes workspace, search/selection, virtualized inventory/references, metadata, age/size sorting, measured-size cache, single/bulk removal and prune review; Docker discovery/size gathering/removal remains in `src/volumes.rs` |
| `container_logs.rs` | Container log history, status, follow/scroll state, and rendering |
| `favorites/` | Favorite command dialog/editor, service and replica selection, bounded serial file worker, guarded manual terminal launch |
| `terminal.rs` | Embedded terminal session state and toolbar/view routing; the emulator remains in `src/terminal.rs` |
| `projects.rs` | Compose associations, editable forms, operation state, and project controls |
| `project_cleanup.rs` | Kit teardown review, volume/image retention options, captured configuration/membership and stale-review guards |
| `project_logs.rs` | Combined project log history, service filtering, follow/scroll state, and rendering; streaming remains in `src/project_logs.rs` |
| `navigation.rs` | Keyboard shortcuts and transitions between feature panels |
| `events.rs` | Worker event dispatch and shared session/selection invariants |
| `shell.rs` | Window layout and placement of feature panels |
| `ui.rs` | Semantic palette, shared controls, empty states, notifications, and dock sizing |
| `appearance.rs` | Persistent theme preference, Kit Settings page, and focus restoration |
| `resource_menu.rs` | Container/project popup menus, sharing the guarded feature actions |

Each feature owns a state type with its startup defaults. The application keeps
shared connection, focus, and navigation state, along with those feature states.
Feature methods still operate on the same GPUI entity so existing cross-feature
selection and session rules have one authoritative application state. Module
internals use `pub(super)` visibility.

Add new features in their own modules with a state type, controls, and view. Wire
them into the application and worker protocol at the shared boundaries. Keep
feature policy out of the application constructor and window layout. If event
dispatch starts collecting substantial feature-specific logic, move that logic
into handlers in the owning module. Extract a separate GPUI entity when a panel
needs an independent lifecycle, rather than splitting components merely to meet
a line count.

Compose source metadata lives in `src/compose/source.rs`. Each discovery round
resolves Docker's labeled files and validates automatic associations in
`spawn_blocking`; saved endpoint/project configurations take precedence.
Worktree detection verifies the primary file's ancestor Git metadata, common
directory and checkout back pointer, distinguishing linked checkouts from
ordinary repositories and separate Git directories. Rendering uses the worker
snapshot without filesystem reads. Automatic associations are rediscovered;
only explicit Configuration saves persist settings and profiles.

Project cleanup uses the same cancellable Compose worker and operation output as
other controls. It runs `down --remove-orphans` with all profiles, adding
`--volumes` only when selected. Image cleanup first resolves the Compose model
and existing generated image names, then removes those explicit names without
force after teardown. Plan reads have a 15-second timeout, a 4 MiB stdout cap,
64 KiB stderr cap and at most 2,000 services. It avoids Compose's version-dependent
`--rmi local` behavior, which can delete custom-tagged project builds. It never
prunes the daemon or deletes bind mounts. External resources and explicit image
tags are preserved. Only successful teardown forgets that endpoint/project's
saved association; failures preserve settings and output for inspection.

`src/metrics.rs` samples running containers through a separate Tokio worker task,
with up to 16 concurrent one-shot Docker stats requests, a two-second pause between
rounds and three-second request timeouts. Inspect-derived limits refresh every
30 seconds. The UI retains at most 60 samples per running container, charts the
last two minutes, and expires readings after eight seconds using Docker's sample
timestamp. Memory usage follows Docker CLI's cgroup v1/v2 cache subtraction;
CPU uses counter deltas and online CPU count, so it can exceed 100%. Worker session
IDs reject readings from an earlier connection; discovery drops stopped/removed
container histories and counter resets clear history after a restart.

Resource plots use Kit `AreaChart` with natural curves, a zero baseline, labelled
auto-scaled CPU-percent/MiB axes and interactive crosshairs/tooltips. Tooltips read
the actual sample, while the curve is visual interpolation. Memory chart scale
follows usage; configured limits and utilization stay in the footer. The latest
contiguous history spans the plot width, up to two minutes; missing readings or
collection gaps start a new plotted series. A container dock defaults to 430px
and has a 400px minimum to retain room for the taller charts and tab content.

`src/images.rs` discovers images and all container references by immutable image
ID in a separate cancellable Tokio task. It refreshes on daemon changes, manual
requests, and a ten-second interval only while the Images workspace is subscribed.
Discovery has a 15-second timeout and retains at most 20,000 images and 50,000
container references; an oversized inventory fails explicitly rather than
truncating removal references. Both lists are virtualized, reference snapshots
are shared across rendered frames, and snapshot generations reject stale results.
Images has its own search, selection, and dock state; opening it preserves the
container/project log and terminal sessions. Kit header buttons sort the visible
indices by case-insensitive image name/ID, age using raw creation timestamps,
size bytes, or container count. Age cells use relative labels; ascending age
sorts newer timestamps first. The detail panel retains the exact UTC timestamp.
Cached sort keys avoid repeated name allocations during comparisons; ID ties
keep ordering deterministic, and unavailable sizes/dates remain last in both directions.
Search and fresh snapshots reapply the session's chosen sort without replacing
the selected image ID. Ctrl-click selection uses an ID set independently of the
active detail image; filtering retains hidden selections and a Kit modal dialog
lists the exact captured IDs. Cancel/Escape closes it, and stale discovery or
disconnects invalidate the confirmation. Removal uses a separate operation ID and a fresh
inventory. Batches remove images sequentially with `force=false` and `noprune=true`,
continue after individual failures, and show completion counts with at most three
failure examples in the existing notification system. No removal-results panel
or retained UI report is added to the Images workspace. Docker
also enforces references created after preflight. Failed images remain selected
if they still exist; removed IDs are pruned without automatically selecting other
images. Compose operations and image removal are mutually exclusive in both UI
and worker guards. Disconnects invalidate operation results and advise checking
daemon state after reconnecting, since a batch may have partially completed.

Image dock tabs live in `src/app/image_details.rs`. History and inspection load
on demand through one cancellable selected-image worker, independently of the
inventory. Selection, refresh, workspace closure and disconnect invalidate
detail generations; both image ID and generation guard completions. Reads have
15-second timeouts and a 4 MiB retained-data limit; history also caps at 2,000
entries. `src/app/image_history.rs` uses Kit DataTable for virtualized rows and
resizable columns in #/Command/Size/Created/Layer ID order. Its Kit Columns menu
preserves selected rows and caches resize events by column identity, so widths
survive hiding fields, filtering and image selection. History search filters
commands while retaining original step numbers. Home/End capture the Kit actions
to select first/last rows rather than columns. Double-click, Enter and the row
menu open the complete instruction in a read-only Kit Editor dialog. Copy command
retains original bytes, including multiline instructions; the table displays a
single clipped line. Data is shared with the detail state through Arc. Inspect
reuses Kit's read-only JSON Editor.

Image Analysis is explicitly requested for the selected immutable ID. Its
worker exports through Bollard to a scoped temporary archive, then runs the
standalone core in `spawn_blocking`. A worker-local semaphore permits one parser;
cancelled readers keep their permit and temporary directory until they exit.
Dropping the request signals cooperative cancellation at metadata entries,
64 KiB decoded reads and accounting checkpoints. A two-minute timeout, 2 GiB
export cap and 16 MiB serialized report cap fail visibly. The core bounds
history/layers at 2,000, archive and layer entries at 30,000, path depth at 64,
individual paths at 4,096 bytes, aggregate paths/JSON at 4 MiB, accumulated
file-tree nodes at 60,000 and decoded layer reads at 8 GiB, with a 128 MiB Zstd decode window. Sparse files, unsafe
or non-UTF-8 paths, multiple manifests and unresolved/cyclic hard links fail
explicitly. The core reads uncompressed outer Docker save archives with plain,
Gzip or Zstd layers; it does not extract files or connect to a registry. Waste
counts regular file payloads with no final path, sharing hard-link payloads
without double counting. Directory metadata retains children; whiteouts apply
to lower-layer state before current-layer additions. No content hashing occurs.

The UI retains one report, with independent request generations, virtualized
layer/file lists, and Kit Tree/TabBar/Resizable/Alert controls. Zero-payload
images have unavailable efficiency. Analyze/Cancel, tree expansion, findings,
errors/retry, stale results, keyboard tabs, large lists and Light rendering are
covered by `python3 scripts/verify-headless.py --flow analysis`. The separate
`bash scripts/verify-image-analysis.sh` checks the production worker using an
owned labelled Alpine image with a deleted 1 MiB file and scoped image cleanup.
These checks do not prove a combined native desktop Analysis session.

The Images toolbar also opens a Kit prune dialog with untagged/all-unused modes.
`PruneImages` runs Docker's prune API in a separate cancellable worker job with a
120-second timeout; explicit `dangling=true/false` filters preserve Docker's
running/stopped-container safeguards. It operates across the connected daemon,
independent of search/selection. UI and worker guards exclude concurrent image,
volume, Compose and container operations. Operation IDs reject stale completion;
notifications report Docker's reclaimed bytes in MiB (or unavailable), and fresh
discovery reconciles selection afterward. `tests/images_prune.rs` exercises the
production API request/response against an owned temporary Unix socket fixture,
without connecting to the user's Docker daemon.

Network discovery joins Docker's network list with all container summaries by
network ID, falling back to name for stopped endpoints without an ID. It retains
at most 20,000 networks, 50,000 containers and 100,000 configured references.
Selected details inspect the network and its containers, with at most sixteen
concurrent reads and three-second per-container deadlines inside a fifteen-second
overall deadline. Partial endpoint errors stay on their rows. Aliases and DNS
names come from container inspect; absent and empty lists remain distinct.
Inventory and detail subscriptions poll every ten seconds, cancel in-flight work
on request changes, and stop when the workspace closes. Independent generations,
selected IDs and connection state reject obsolete results. Endpoint changes
clear inventory/selection and container scoping. Lists are virtualized; a cached
set of locally discovered container IDs guards navigation without per-row scans.

## UI components and appearance

Use GPUI Kit for standard controls, navigation, forms, feedback, and settings.
Keep the virtualized ANSI log views and Alacritty terminal rendering specialized.
Inspect uses Kit's read-only JSON editor with syntax highlighting and search.
Initialize Kit and its assets before constructing the application's Root view.

Kit's `gpui-fast` feature and all direct GPUI engine/platform/WGPU dependencies
must select the same backend so their public types remain compatible. The
versions and features are pinned in [Cargo.toml](../Cargo.toml); `tree-sitter`
enables the JSON grammar, and `windows-manifest-provided-by-host` avoids
supplying a second Windows manifest alongside Kit.

Appearance uses Kit's semantic theme tokens. Settings offers FerryDock's
charcoal/mint default, Dark, and Light. Update all views from the same palette,
including log and terminal defaults; preserve explicit ANSI backgrounds and
keep bright text readable on Light surfaces.

Preferences are saved in `$XDG_CONFIG_HOME/ferrydock/appearance.json`, falling
back to `~/.config/ferrydock/appearance.json`, separately from Compose settings.
Missing or invalid preferences fall back to FerryDock; read/save errors appear
in Settings. A failed save leaves the chosen theme active for that session.
Verification uses an isolated preference file.

## Docker and Compose boundaries

The container list keeps Compose and standalone fold state in `Containers` for
the application session. Kit buttons and ListItem rows form bordered groups;
the virtualized list omits folded members instead of mounting each project's
contents in an Accordion/Collapsible. Search temporarily expands matching groups
without changing stored folds. Keyboard selection uses the same expansion rule;
folding never changes the detail selection or opens/closes worker streams.

Identify Compose membership from project/service labels and scope saved
associations by Docker endpoint and project name. Labels and file suggestions
cannot reconstruct the original environment and profile choices. Persist
explicit associations so projects remain available without containers.

Run Compose v2 with structured arguments, preserving ordered files, environment
files, profiles, and the displayed endpoint. Stream output, retain failures,
and reject conflicting container/project operations. Cancelling or closing
terminates the tracked process; it does not roll back Docker effects.

Keep service and project action scope explicit. Standalone containers have no
implied build recipe. Stop preserves data; Stop & cleanup requires a review with
visible volume/image deletion choices and a separate confirmation. Account for
references from stopped containers when presenting volume removal, and reconcile
daemon state after operations.

`src/volumes.rs` discovers volumes and all container mounts through a cancellable
Tokio task. Discovery times out after 15 seconds and refuses inventories larger
than 20,000 volumes, 50,000 containers, or 100,000 volume/container references.
Docker inventory warnings and unnamed volume mounts fail explicitly. References
include stopped containers and deduplicate repeated mounts of one volume in a
container. Ordinary discovery does not request disk usage. Only
`GatherVolumeSizes` requests `GET /v<negotiated>/system/df?type=volume&verbose=true`,
using the current local endpoint and API 1.42+ so image/container/build-cache usage
is excluded. This separate cancellable worker job has a 120-second timeout,
32 MiB response bound and 20,000-volume bound. Bollard's generated disk-usage model
only decodes the newer schema; the feature reuses its already locked Hyper
Unix-socket/named-pipe transports to decode both legacy `Volumes` and modern
`VolumeUsage.Items`. `src/docker_transport.rs` owns those local transports;
Bollard negotiates the API version on the selected endpoint. Remote transports
remain unsupported.
Missing/negative sizes are unavailable; zero is measured empty. Sizes are labelled
in MiB and sorted by bytes, with unavailable values last in either direction.

The app-session cache is separate from inventory events. Refresh replaces usage
fields with cached measurements matched by name, driver, mountpoint and creation
time; vanished/recreated volumes lose their entries. Successful gathering replaces
measurements and the UTC completion timestamp; failure preserves both. Workspace
changes and same-endpoint reconnects retain them; a different endpoint clears the
cache. An independent request ID rejects late completions after disconnect, and
closing the workspace does not cancel an explicit gather.
The view uses Kit Input, ListItem, Dialog, Toolbar and Resizable controls; inventory,
reference and metadata lists are virtualized. Generations and operation IDs reject
stale results; disconnect invalidates review and pending state. Removal performs
one fresh inventory per batch, then sequential `remove_volume(force=false)`
requests with a 30-second timeout each. Batches continue after individual failures.
Images and Volumes share elapsed-time wording through `src/resource_age.rs`.
The selection uses a name set independent of the active detail volume. Kit header
buttons sort name, driver, raw creation time (Age), measured bytes (Size), or
reference count; unavailable ages/sizes stay last in either direction. Cached sort keys and virtualized rows bound
rendering work. Hidden selections appear in the exact-name confirmation; successful
results leave selection and failures stay selected while they remain in Docker.
The UI and worker guard concurrent container, image and Compose mutations during
removal or pruning. Other clients remain protected by Docker's reference checks.

Volume prune is a separate explicit operation, with a Kit review dialog offering
Anonymous or AllUnused and a generic Enter guard. `PruneVolumes` sends the chosen
mode to the production worker; `all=false` preserves named volumes and `all=true`
includes them. Both require API 1.42+ so older daemon semantics cannot accidentally
prune named data in anonymous mode. Search and selection never become prune
filters. A separate cancellable worker job uses a 120-second Docker/overall
timeout and shares the mutation guard with image prune, removals and container/
Compose controls. Completion reports returned deleted names and reclaimed bytes;
missing/negative space is unavailable, and zero stays zero. The view immediately
reconciles reported deletions and cached-size/selection state, then refreshes
ordinary discovery without recalculating surviving sizes. Operation IDs and
session invalidation reject stale/disconnected completions; navigation keeps an
accepted prune running. Production daemon-wide prune effects are not executed by
the verification fixtures: isolated Unix API requests and GPUI input/fixture
results verify the two sides of this boundary.

## Build on Linux

Install Rust through rustup and the native build dependencies. The toolchain is
pinned to Rust 1.99.0 in `rust-toolchain.toml`. On Debian/Ubuntu, the typical
prerequisites are:

```sh
sudo apt install build-essential pkg-config cmake clang \
  libssl-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libfontconfig1-dev libfreetype-dev \
  libxcb1-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev
```

GitHub Actions builds and tests on Ubuntu 24.04 with these dependencies.

`Cargo.lock` is included. For the initial dependency download:

```sh
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo fetch --locked
```

For interactive development, reuse the incremental debug build:

```sh
cargo run --locked
```

For a release build:

```sh
cargo build --release --locked
./target/release/ferrydock
```

Use a desktop terminal with access to Docker and a GPU/driver supported by GPUI.
The Linux app lets GPUI select native Wayland when `WAYLAND_DISPLAY` is set,
or X11/XWayland when only `DISPLAY` is set. Launch from the desktop session
without changing these variables. To exercise X11 explicitly on a Wayland
desktop, use `env -u WAYLAND_DISPLAY cargo run --locked`; this override is
limited to that child process. Backend selection does not retry X11 after a
Wayland connection failure.

## Build on macOS

The macOS package targets Apple Silicon and macOS 15 or newer. Install Rust
through rustup and Xcode, select the Xcode developer directory with
`sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`, and complete
Xcode's first-launch setup. GPUI's native Mac backend needs the Metal compiler
from Xcode. Use the pinned toolchain and locked dependency graph:

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build --release --locked --target aarch64-apple-darwin
./target/aarch64-apple-darwin/release/ferrydock
bash scripts/package-macos.sh
```

Packaging runs on an Apple Silicon Mac and writes
`target/packages/ferrydock-<version>-macos-arm64.dmg` and `SHA256SUMS`.
It creates an app bundle, ad hoc signs the app and DMG, verifies signatures,
then mounts the finished DMG to check its app, architecture, signed binary,
and install files. Non-system dylib dependencies fail packaging so they cannot
silently escape the bundle. The package contains an Applications shortcut,
license, and instructions for approving the unnotarized app's first launch.
No signing certificate, Apple account, or notarization secrets are required.

`packaging/macos/FerryDock.icns` is generated from the existing Linux SVG icon.
To regenerate it on Linux with `rsvg-convert` and Python's Pillow installed:

```sh
rsvg-convert -w 1024 -h 1024 packaging/linux/dev.ferrydock.FerryDock.svg -o /tmp/ferrydock-icon.png
python3 -c "from PIL import Image; Image.open('/tmp/ferrydock-icon.png').save('packaging/macos/FerryDock.icns', format='ICNS')"
```

Finder does not inherit a terminal's PATH. Mac Docker context lookup and Compose
commands append user Docker, Homebrew, and Docker Desktop CLI directories only
to their child environments, preserving inherited PATH priority. Log and
terminal views use the system Menlo font on macOS. Existing Control shortcuts
and settings paths are retained.

## Build on Windows

The preview targets Windows 10/11 x64 and Docker Desktop's Linux containers.
Install Rust through rustup, Python 3.11+, and Visual Studio 2022 Build Tools
with **Desktop development with C++** and a Windows SDK. The SDK supplies
`rc.exe` for application resources and `fxc.exe` for GPUI's release shaders.
Build from a Windows terminal with the pinned toolchain:

```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --release --locked --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\ferrydock.exe
python scripts/package-windows.py
```

The portable ZIP, `SHA256SUMS` and package inspection report go to
`target/packages/`. Packaging checks the x64 PE architecture, GUI subsystem,
icon/manifest resources, normal/delayed DLL imports and ZIP contents. The target
configuration links the C runtime statically; packaging rejects VC runtime DLLs
and missing non-system dependencies. The executable is unsigned. The icon is
converted from the existing SVG. To regenerate it on Linux with ImageMagick and
Python's Pillow installed:

```sh
magick -background none packaging/linux/dev.ferrydock.FerryDock.svg target/windows-icon.png
python3 -c "from PIL import Image; Image.open('target/windows-icon.png').save('packaging/windows/FerryDock.ico', format='ICO', sizes=[(n,n) for n in [16,24,32,48,64,128,256]])"
```

The pinned Kit base layer's `gpui-pre-platform` dependency already embeds a
manifest with per-monitor DPI awareness and current-user execution. FerryDock
embeds only its icon to avoid duplicate manifest resources during native linking;
the package inspector requires both the icon and the inherited manifest.
Docker CLI subprocesses use `CREATE_NO_WINDOW`; logs and terminals use Consolas
through the existing Kit styling. Associations, favorites and appearance share
`%APPDATA%\ferrydock`; Unix paths and defaults remain unchanged. Settings use
temporary files in the destination directory for atomic replacement. Windows
Docker contexts must expose a local `npipe://` endpoint. Compose commands pass
the selected endpoint explicitly and require Windows-accessible file paths;
Linux paths from WSL labels may need manual association. Bash/sh run inside
Linux containers; Windows container shells are outside this preview's scope.

Run `cargo test --workspace --locked --target x86_64-pc-windows-msvc` for unit
tests and owned named-pipe fixtures. `local_connection` tests production Bollard
API negotiation; `volume_sizes` tests the filtered HTTP request and both response
schemas. No Docker daemon is contacted by those fixtures. The Linux-only
headless/native helpers do not establish Windows interaction or Docker Desktop
compatibility. Before promoting the preview, verify on a Windows desktop:

1. Extract the ZIP on a machine without the VC redistributable; launch it and
   inspect the icon, default theme, Consolas text and rendering across DPI scales.
2. With Docker Desktop in Linux containers mode, use an owned disposable Compose
   project without host mounts, published ports or persistent volumes. Verify
   discovery, logs, terminal input/PTY size and an independently read container file.
3. Associate Windows-local Compose files, exercise Start/Stop/Restart and confirm
   real daemon state changes. Check CLI operations produce no console windows.
4. Save/reopen settings and favorites, exercise clipboard and focus, disconnect
   and reconnect Docker, then remove only the owned fixture. Retain screenshots,
   action results and cleanup evidence under `.verification/windows/`.

## Verification

Run checks from the repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --lib --locked
python3 -m unittest discover -s tests -p 'test_*.py' -v
python3 scripts/verify-headless.py
```

The headless flow uses the real FerryDock view, WGPU renderer, and GPUI keyboard
and pointer events. It needs Python 3.11+, cached Cargo dependencies, and a
usable Linux graphics driver. It removes display variables only from its child
and supplies fixture events at the Docker worker boundary. No desktop window
or Docker worker starts. The optional `headless-tests` feature stays disabled
in normal builds.

Coverage includes search/editing, resource menus, themes and persistence,
log scrolling/ANSI rendering, Inspect search/read-only behavior, resizing,
notifications, terminal input routing, Compose forms/requests, stale-event
guards, and bounded history under a 10,000-record burst. Fixture requests prove
UI behavior; real operations require the separate integration checks:

| Command | Boundary exercised |
| --- | --- |
| `bash scripts/verify-docker.sh` | Real discovery/logs, Start/Stop/Restart, interactive shell I/O, PTY resize, ANSI parsing, full-screen vi, shell exit, missing Bash |
| `bash scripts/verify-compose.sh` | Production worker/Compose CLI, saved configuration and override order, environment/profiles, lifecycle effects, changed build inputs and rebuilt image/container contents, build failure without stopping services, streamed failures, restart/rebuild/recreation log recovery, bounded batches |
| `cargo test --locked --test volumes_prune` | Owned Unix HTTP fixtures: exact anonymous/all-unused filters, deleted names and reclaimed bytes, empty/zero/unavailable results, Docker errors and old-API refusal |
| `cargo test --locked --test local_connection --test volume_sizes` | Owned local HTTP fixtures (Unix sockets or Windows named pipes): API negotiation, volume-only query and legacy/modern response decoding, zero/unavailable values, errors and API version guard |
| `bash scripts/verify-volumes.sh` | Explicit volume-only disk usage of a known 3 MiB file; production worker volume inventory/configuration, running/stopped references and deletion guards, single/bulk unused-volume deletion, per-target failures and reconciliation |
| `bash scripts/verify-images.sh` | Production worker image discovery, immutable IDs and running/stopped references, blocked removal, multi-tag Docker conflict, selected-image removal and inventory reconciliation |
| `python3 scripts/verify-log-performance.py` | Owned native window (Wayland preferred) and ten disposable containers; CPU/RSS and search under live log load |

Docker checks create unique Compose fixtures on the existing daemon and remove
only their containers and networks. The Compose check also removes its uniquely
labelled built images, including superseded images; Docker's build cache and the
`alpine:3.23` base image remain. Compose settings and build inputs are isolated
under the evidence directory. Fixtures
have no host mounts, published ports, or persistent volumes.

The image check owns four uniquely labelled/tagged images and one container with
networking disabled. It removes only those resources and preserves the Alpine
base image and Docker build cache. Settings and evidence are isolated under
`.verification/images/`. Headless image checks drive sidebar navigation, tag/ID
search, a 10,000-image virtualized inventory, removal review/cancellation,
stopped-container reference guards, success/failure and stale-event handling,
rendered column alignment, and Light theme. These prove GPUI input/rendering
and worker requests, separately from the real Docker integration check; native
desktop Images interaction and OS clipboard behavior remain unverified.
The headless flow also drives Ctrl-click toggles, clearing the last selection,
selection retention through sort/search/refresh, hidden-target review, partial
bulk results and full success. The Docker fixture checks mixed batches with
stopped-container/multi-tag refusals and successful two-image deletion.

The volume check creates five uniquely named/labelled local volumes and one
container using `alpine:3.23` with networking disabled. It has no host bind mounts
or published ports and deletes only its owned container and volumes; the base
image remains. Its used volume contains a known 3 MiB file; one explicit read-only
size calculation covers all eligible volumes on the shared daemon, while mutation
and cleanup stay restricted to owned fixtures. Settings/evidence are isolated under
`.verification/volumes/`. Normal discovery is checked to emit no size results.
For a focused UI loop, run `python3 scripts/verify-headless.py --flow volumes`;
the default full application flow also includes volume checks.
Headless volume checks drive sidebar/search, stopped-reference navigation,
Cancel/Escape, exact-name confirmation, pending/success/failure, references
appearing during review, stale results, disconnection, Light theme and a
10,000-row inventory. Header sorting and age-column alignment, Ctrl-click toggles,
empty/hidden selections, refresh retention, partial/full batch results and bulk
stale/disconnected events are also driven. Size checks click Gather, guard duplicate
requests, sort measured bytes with unavailable last, retain measurements through
refresh/navigation/same-endpoint reconnect, preserve them on failure, invalidate
recreated/other-endpoint entries and reject interrupted/stale gathers. The Docker
fixture checks mixed
referenced/missing/unused batches and successful two-volume deletion.
Headless prune checks cover both modes with search/selection independence,
Cancel/Escape and generic Enter protection, loading/duplicate and conflicting
container/image controls, success/failure, zero/unavailable reclaimed space,
survivor cache/selection retention, workspace changes and stale/disconnect guards.
The isolated prune API fixtures never contact the user's daemon.
These prove GPUI input/rendering and worker requests
separately from daemon behavior. Native desktop Volumes interaction and macOS
runtime remain unverified.

Build with `cargo build --locked` before the native performance helper; use
`--binary target/release/ferrydock` after a release build to measure shipping
performance. `--ansi` emits coloured UTF-8 records; `--profile` captures UI-thread
stacks through GDB. The helper takes desktop focus and must not overlap another
native-input run. Measurements depend on host, build profile, and workload;
capture/OCR overhead is reported separately.

The default `--backend auto` selects native Wayland when `WAYLAND_DISPLAY` is
set, otherwise X11. Use `--backend wayland` to require Wayland, or
`--backend x11 --display :0` for an explicit XWayland comparison. A missing
Wayland prerequisite fails the run instead of silently switching backends.
The demonstrated Wayland adapter requires KDE/KWin, `kdotool`, `qdbus6`,
Spectacle, `ydotool`, `ydotoold`, and access to `/dev/uinput`; other compositors
need a separate adapter. It starts its own temporary keyboard daemon/socket,
checks window PID and focus before input/capture, and removes those resources
along with its application and Docker fixture. It does not change system input
permissions. X11 uses `xdotool` and ImageMagick. Both backends use Tesseract.
The result records the actual backend, child display environment, input/capture
methods, and input cleanup. Search timings include backend-specific automation
overhead, so compare CPU/RSS separately from input/capture timings.

For desktop feature checks, use the project-local
[verify-ferrydock skill](../.agents/skills/verify-ferrydock/SKILL.md) and its
[feature map](../.agents/skills/verify-ferrydock/features/README.md):

```sh
cargo build --release --locked
python3 .agents/skills/verify-ferrydock/scripts/native.py doctor --display :0
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature terminal --display :0
```

The helper owns its application window and disposable fixture, verifies
selection before input, and performs scoped cleanup. It requires an accessible
X11 display, `xdotool`, ImageMagick, Tesseract, and Python with Pillow. Use the
actual display on your workstation. See the
[verification feature map](../.agents/skills/verify-ferrydock/features/README.md)
for entry points and the coverage boundaries of each flow.

Headless checks establish GPUI input and GPU rendering using fixture worker
events. Docker and Compose integration checks establish daemon/CLI behavior
separately. Neither proves native desktop input or operating-system clipboard
behavior. The native log-load helper covers log reading and search on its
supported backend; service-filter/pause clicks and clipboard behavior need
separate native proof. A scripted helper or CI definition alone is not evidence
that a flow passed. Report the actual checks exercised for each change.

Headless evidence lives under `.verification/headless/`, Compose results under
`.verification/compose/`, native feature results under `.verification/ferrydock/`,
and load measurements under `.verification/log-performance/`. A passing result
requires successful owned-process/fixture cleanup as applicable. Evidence is
local and ignored by Git.

Headless/native helpers keep the newest eight completed default-path runs.
Active runs, failed cleanup, explicit `--evidence` directories, and `.keep`-pinned
runs are preserved. `--keep-runs N` changes retention; `--keep-runs 0` disables
pruning. Preview existing-run pruning with
`python3 scripts/verification_evidence.py --dry-run`; omit `--dry-run` to apply it.
Pin or export evidence used by durable reports.

Terminal exec creation includes Docker's initial `ConsoleSize` before starting
an interactive shell or favorite command. Later resize requests follow window
changes. Avoid a mandatory resize after start: an immediate command can already
have exited, and its buffered output must still be read. The two
`scripts/verify-docker.sh` tests run serially because they share one owned fixture;
its favorite check covers initial PTY size, interactive input, occupied-session
protection, independent file proof and immediate-command output/exit.

For the focused Networks input/rendering flow, run
`python3 scripts/verify-headless.py --flow networks`. It exercises search,
selection, configured stopped endpoints, full-value copying in the virtual
clipboard, errors/retry, stale guards, container routing and large-list
virtualization. Inspect its endpoint, scoped-network and Light screenshots.
`bash scripts/verify-networks.sh` separately verifies production worker reads
against two uniquely labelled bridge networks and two Alpine containers, one
stopped. The fixture has no published ports, host mounts or volumes. Its test
disconnects only its owned running endpoint to verify membership reconciliation;
cleanup removes only the fixture's labelled containers/networks. Require
`.verification/networks/<run>/result.json` with `status: passed` and
`cleanup_success: true`. Headless screenshots do not prove native desktop input;
worker tests do not prove user-interface interactions or DNS reachability.

## CI and releases

[Tests](../.github/workflows/ci.yml) runs on pushes to `main`, pull requests,
and manual dispatches. It checks formatting, Clippy, Rust and Python tests, the disposable
Docker and Compose integration fixtures, and the headless UI flow with Mesa's
software GPU driver. Headless and Compose evidence are retained as workflow
artifacts for seven days.

Pull requests run the full Linux suite only; newer commits cancel superseded
runs for the same PR. Pushes to `main` also run macOS and Windows compilation
and tests, without release builds or packages. Manual runs include both platforms
and default to building their packages; uncheck **build-packages** in **Run workflow**
to run checks only. Select the relevant branch for platform checks before merging.
Release tags explicitly enable packaging on both platforms.

The Images and Volumes worker integration fixtures also run in Linux CI and retain their
discovery/removal/cleanup evidence for seven days.

The reusable [macOS workflow](../.github/workflows/macos.yml) uses the
`macos-15` Apple Silicon runner. It runs Clippy and unit tests. When packaging
is enabled, it also builds the release binary and packages/verifies the ad hoc
signed DMG, producing the `macos-arm64-packages` artifact. These checks
do not launch the native Mac UI, connect to Docker Desktop, or exercise
Gatekeeper approval. The existing headless/native verification helpers are
Linux-only; their passing results do not prove macOS runtime behavior.

The reusable [Windows workflow](../.github/workflows/windows.yml) runs on
`windows-2025` with the x64 MSVC target. It runs Clippy, unit tests and isolated
named-pipe API fixtures, and package regression tests. When packaging is enabled,
it also builds the release executable and inspects the portable ZIP.
`windows-x86_64-packages` contains the ZIP and checksums;
`windows-package-check` records the executable/import/archive checks. These jobs
do not launch the native UI or contact Docker Desktop. Windows previews remain
pending the desktop verification described above.

[Release](../.github/workflows/release.yml) runs when a `v*` tag is pushed. It
reuses the test workflow, including macOS and Windows packaging, then builds Linux x86-64
packages on Ubuntu 24.04:

- A `.tar.gz` archive containing the binary, license, install guide, and desktop
  launcher.
- An `.deb` package with library dependencies derived from the built binary.
- An `.AppImage` with ELF dependencies, an icon/launcher, and bundled library
  copyright notices. `linuxdeploy` collects dependencies, patches library search
  paths, copies Debian library copyright notices, and installs desktop assets;
  `appimagetool` packages the result. glibc, fonts, Wayland libraries, and the
  Vulkan/EGL graphics driver stack remain host requirements. Wayland libraries
  must match the host drivers; an older bundled client library can prevent newer
  Mesa drivers from loading.
- The macOS workflow supplies the Apple Silicon `.dmg`.
- The Windows workflow supplies the x64 preview `.zip`.
- The publish job verifies each platform's checksums and generates one
  `SHA256SUMS` covering all five versioned downloads and their stable aliases.
  It requires Linux, macOS and Windows checks/packages to succeed before
  creating the release.

The workflow installs the Debian package and checks both binaries in a clean
Ubuntu container before publishing the GitHub Release. This startup check
verifies library loading and the missing-display error; it does not exercise
the native desktop UI. A separate clean Ubuntu container launches the AppImage
with `--appimage-extract-and-run` on its own Xvfb display using Mesa software
Vulkan. It checks that the AppImage opens its own visible window and loads
application libraries from the bundle. The AppImage check has no Docker socket
and does not test UI interactions, Docker operations, native Wayland, FUSE
mounting, or hardware GPU compatibility. Its library-loading evidence, logs,
and result are retained in the `appimage-proof` workflow artifact.
See the [verification feature map](../.agents/skills/verify-ferrydock/features/README.md)
for native UI entry points and coverage boundaries. Keep individual run reports
with their ignored `.verification/` evidence; a local consolidated report may
be kept at `.verification/VERIFICATION.md`. Include relevant validation results
in PR descriptions and retain CI evidence as workflow artifacts.

For a release, update the Cargo version and lockfile, add user-facing notes in
`docs/releases/v<version>.md`, commit the changes, and push the matching tag:

```sh
git tag -a v0.7.0 -m "FerryDock 0.7.0"
git push --atomic origin main v0.7.0
```

Replace the example version with the version being released. The tag must match
the Cargo version and have a release-notes file. The publish
job is the only job with write access to repository contents.

To package a locally built release on an Ubuntu 24.04 x86-64 host, install
`dpkg-dev`, `curl`, `file`, Python 3, and the Linux build dependencies, then
build the binary and run:

```sh
bash scripts/package-linux.sh v0.7.0
```

Output goes to `target/packages/`. Local builds on newer distributions may
require newer runtime libraries; publish the Ubuntu-built workflow artifacts.
The linuxdeploy release, AppImage builder, and runtime are pinned and SHA-256
checked by `scripts/package-appimage.sh`, with downloads cached in
`target/appimage-tools/`. linuxdeploy uses Debian package metadata to retain
library copyright notices under `usr/share/doc/<library-package>/copyright`,
so run packaging on Ubuntu/Debian.
`libxcb` is explicitly bundled to preserve startup on a clean Ubuntu system;
Wayland and graphics loaders are explicitly excluded to use the host driver stack.

To reproduce the AppImage check without exposing the host Docker daemon:

```sh
docker build -f packaging/linux/appimage-test.Dockerfile -t ferrydock-appimage-test packaging/linux
mkdir -p target/appimage-proof
docker run --rm --user "$(id -u):$(id -g)" -v "$PWD/target/packages:/packages:ro" \
  -v "$PWD/scripts/verify-appimage.py:/verify-appimage.py:ro" \
  -v "$PWD/target/appimage-proof:/proof" ferrydock-appimage-test \
  bash -euc 'python3 /verify-appimage.py /packages/ferrydock-*.AppImage --evidence /proof/run'
```

Use a new evidence directory for each run; the verifier refuses an existing one.

## Favorite command verification

`src/favorites.rs` owns endpoint/target recipes and private atomic JSON replacement.
The feature's serial file worker loads and mutates settings off the UI thread.
Files cap at 2 MiB / 2,048 recipes, labels at 120 bytes and commands at 16 KiB.
The manager virtualizes recipes; the terminal uses Kit Select with stable recipe
IDs. Selection is scoped to the current container's endpoint and service.
`favorites/quick_run/` keeps the Kit List launcher, replica picker and interactive
console together. Recent-use and pin metadata use backward-compatible defaults
in the same file worker. Project headers show at most three pins. Quick PTYs have
a separate generation namespace and a six-session cap in both UI and worker;
normal Terminal selection/disconnection cannot replace them. Pending commands
expire after 15 seconds and recheck endpoint/target/operation state before sending.
Closing the console preserves sessions; End session releases one explicitly.

`src/terminal_session.rs` owns the Docker PTY and session-specific prompt OSC.
Bash's prompt hook or sh's prompt expansion returns the preceding command's exit
status. The bounded split-frame decoder consumes only that session's marker.
`RunFavorite` writes to the existing shell only after idle/session/endpoint/target
checks. Multiline recipes form one shell group, retaining cwd and exports. Input
marks the session busy immediately; terminal protocol replies have a separate path.
UI run IDs and input counters reject stale completions and prompt observations.
Bash loads the normal user `.bashrc` through an inherited pipe before installing
the prompt hook. Existing scalar or array `PROMPT_COMMAND` hooks are preserved,
including the preceding exit status. No startup file is written in the container.

Run `python3 scripts/verify-headless.py --flow quick-run` for launcher search,
keyboard execution, pins, replica choice, busy-shell preservation, idle reuse,
results and stopped/changed-target guards. The owned Docker test
`bash scripts/verify-docker.sh quick_run_terminals_preserve_interactive_sessions`
corroborates independent regular/two-quick PTYs with separately read files.

Run `python3 scripts/verify-headless.py --flow favorites` for real GPUI input and
screenshots, including running/success/failure, continued use of the same shell,
partial-input protection, manager persistence/focus and scope guards. Preferences
use an isolated evidence path. `bash scripts/verify-docker.sh
favorite_command_uses_an_interactive_pty` exercises the real sh worker in its
owned fixture: PTY dimensions, interactive input/file proof, consecutive commands,
exit 0/1, multiline cwd/export persistence and manual/partial-input rejection.
`FERRYDOCK_FAVORITE_SHELL=bash` selects Bash when supplying a labelled fixture
image containing it; `auto` checks automatic shell selection. Headless input and
Docker behavior are separate proofs; native desktop and macOS need separate
checks. See the [favorites map](../.agents/skills/verify-ferrydock/features/favorites.md).

## Project website

The one-page product site lives in `website/`. It is plain HTML, CSS and
JavaScript with local screenshots; there is no frontend dependency installation.
Preview it with `python3 -m http.server 4173 --directory website`.

The Website workflow uploads `website/` directly to GitHub Pages on website
changes to `main` or manual dispatch. Pull requests upload the artifact without
deploying. Download buttons use GitHub's `/releases/latest/download/<filename>`
redirects. The release workflow publishes versionless aliases alongside the
versioned packages and includes both in `SHA256SUMS`, so new releases need no
website update or deployment.
Enable Pages with **Settings → Pages → Source → GitHub Actions** for the first
deployment. Add new feature screenshots under `website/assets/screenshots/`
as AVIF from original PNG captures, using example projects without private data.

## Dependency licenses

FerryDock is [MIT licensed](../LICENSE). Dependencies retain their own licenses;
package manifests and bundled library copyright notices are authoritative.
No upstream sources are vendored or patched.
