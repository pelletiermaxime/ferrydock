# FerryDock

A native desktop app for managing your Docker containers on Linux, macOS and Windows.

Find a container, follow its logs, open a shell, and start or stop it from one
window. FerryDock groups containers by Docker Compose project and keeps logs,
the terminal, and container details close at hand.

![FerryDock showing a Compose project and an interactive container terminal](docs/screenshots/ferrydock-terminal.png)

## What you can do

- **Find containers quickly.** Browse Compose projects and standalone containers,
  search by name, image, service, or project, and switch between running and all
  containers. A small colour marker beside each name matches its label in project logs.
- **Follow live logs.** Read output with ANSI colours and styles, pause automatic scrolling, copy
  logs, and reconnect the stream. Combine a Compose project's service logs in
  one view and filter them by service. Timestamped Docker markers show container
  starts, stops, restarts, exits, and other lifecycle changes, including actions
  from the terminal or another app. Markers are kept for the current FerryDock
  session; the log status warns when an event-stream interruption may leave gaps.
  Completed restarts appear as one highlighted row with a Docker badge. Hover a
  marker to see its underlying events, signals, and exit codes.
  Stopping a container shows one stop request and one stopped confirmation;
  shutdown signals and process-exit details are grouped into those markers.
- **Monitor CPU and memory.** See live CPU percentage and memory usage/limit in
  the container list, with smooth area charts in the detail dock. Hover for
  measured values and sample age. Labelled axes show CPU percentage and memory
  in MiB, scaled to recent usage; available history spans the chart width, up
  to two minutes. Use **Hide graphs** to leave a compact live summary and give
  more space to logs, or **Show graphs** to expand them again. Your choice stays
  in effect across containers and Logs, Terminal, and Inspect until you close FerryDock.
  CPU 100% means one core; configured CPU quotas are shown separately. Containers
  without a memory limit show Docker capacity explicitly. Missing, stopped,
  disconnected, and stale readings are labelled instead of reported as zero.
- **Open a terminal inside a container.** Use Bash or sh with colors, scrollback,
  text selection, copy and paste, and support for full-screen terminal programs.
- **Keep favorite commands.** Save labelled commands for a Compose service or
  standalone container on the connected Docker endpoint. Review the command,
  choose a running instance when a service has replicas, and run it manually
  in the integrated interactive terminal.
- **Start, stop, and restart containers.** Use the actions at the right of each
  container row or in its detail panel. Row actions keep your current logs or
  terminal open when controlling another container. See progress in the row and
  completion or errors in notifications. Associate Compose files to control a
  whole project, rebuild its images and restart its services, and watch its
  operation output.
- **Inspect container configuration.** Read syntax-highlighted JSON with line
  numbers, text selection, folding, and built-in search. The view is read-only.
- **Manage local images.** Open **Images** to search tags, digests, or image IDs,
  read sizes and creation dates, and see every container using an image,
  including stopped containers. Ctrl-click to select several images, then review
    and remove them together with completion feedback, or prune untagged or all
    unused images.
- **Manage local volumes.** Open **Volumes** to search by name, driver, or Compose
  project, inspect configuration, and find running or stopped container references.
  Review permanent deletion before removing selected volumes, or prune unused
  anonymous volumes or all unused volumes.
- **Explore local networks.** Browse networks, subnets and gateways, then see configured
  containers with their IPv4/IPv6 addresses and DNS aliases. Open a container
  directly or choose **More → View networks** to find its neighbours.
- **Adjust the workspace.** Drag the divider between the container list and
  detail panel. Action buttons show loading state, and notifications report
  completed operations and copy feedback.
  Double-click an image-history column divider to fit its contents. The
  image-analysis layer sidebar keeps its chosen width when the window resizes.
- **Choose your theme.** Open **Settings** from the sidebar or toolbar to choose
  **FerryDock**, **Dark**, or **Light**. The original charcoal and mint FerryDock
  theme remains the default. Changes apply immediately and are remembered on
  this computer; **Use FerryDock theme** restores the default.
- **Update the Linux AppImage.** FerryDock checks GitHub Releases after startup
  and every six hours, downloads newer stable releases in the background, and
  verifies them against the published SHA-256 checksum. Choose **Restart to
  update** when you are ready. **Settings → Updates** includes a manual check
  and an option to turn automatic checks and downloads off.
- **Use resource menus.** Right-click a container or project, or use **More** in
  its detail toolbar, for navigation, configuration, and available actions.
- **Navigate container logs.** Use visible scrollbars for history and long
  lines. Opening a container shows the newest retained output immediately,
  then follows new lines without replaying the initial history.

## Get started

FerryDock is in early development. Packages target **Linux x86-64 with
Wayland or X11/XWayland** and **Apple Silicon Macs running macOS 15 or newer**,
with a **local Docker daemon**. A **Windows 10/11 x64 preview** targets Docker
Desktop running Linux containers. macOS and Windows packages are previews pending
native desktop verification. You need a working graphics driver and access to Docker
from your user account. FerryDock uses your existing Docker installation.

### Install

Download a package from [GitHub Releases](https://github.com/pelletiermaxime/ferrydock/releases/latest).
macOS packages are included starting with `0.5.1`.

For **Windows x64**, download `ferrydock-<version>-windows-x86_64.zip`
(available starting with `0.12.0`). Extract the ZIP and open **ferrydock.exe**.
Start Docker Desktop in Linux containers mode first, and make sure `docker` is available in your
Windows PATH. Project controls also need Docker Compose v2.

The Windows preview is unsigned, so Windows may show a SmartScreen warning.
Verify the ZIP against `SHA256SUMS` with PowerShell's
`Get-FileHash .\ferrydock-<version>-windows-x86_64.zip -Algorithm SHA256`.
Settings are stored in `%APPDATA%\ferrydock`. Compose files must be accessible
to Windows; projects started inside WSL may need a local file association
through **Configuration**. Windows containers and PowerShell/cmd container
shells are outside this preview's scope.

For **Apple Silicon macOS**, download `ferrydock-<version>-macos-arm64.dmg`,
open it, and drag **FerryDock.app** into **Applications**. Start Docker Desktop
or another Docker daemon using a local Unix socket context, then open FerryDock.
Docker Compose v2 is needed for project controls. Intel Mac packages are not provided.

Mac builds are **ad hoc signed and not notarized**. If macOS blocks the first
launch, try opening FerryDock, then go to **System Settings → Privacy & Security
→ Open Anyway** and confirm **Open**. See [Apple's instructions](https://support.apple.com/102445).

The **Linux x86-64** packages are built on Ubuntu 24.04 and require glibc 2.39
or newer.

The examples below use `0.15.0`; replace it with the version you downloaded.
On Ubuntu 24.04 or a compatible Debian-based distribution, install the Debian
package:

```sh
sudo apt install ./ferrydock_0.15.0_amd64.deb
```

Launch **FerryDock** from your applications menu or run `ferrydock` from a desktop
terminal. The package installs its runtime library dependencies automatically.
You still need Docker and a working graphics driver.

For a portable download on a compatible distribution, use
`ferrydock-0.15.0-linux-x86_64.AppImage`:

```sh
chmod +x ferrydock-0.15.0-linux-x86_64.AppImage
./ferrydock-0.15.0-linux-x86_64.AppImage
```

The AppImage bundles application libraries and needs no package installation.
It still requires glibc 2.39 or newer, an installed font, your desktop's
Wayland libraries and Vulkan/EGL graphics driver stack, and Docker. If FUSE
mounting is unavailable, run it with `--appimage-extract-and-run`.

AppImage updates require write access to the directory containing the original
AppImage and enough free space for the download. Applying an update preserves
that file's location and name. Open terminal and quick-run sessions close when
FerryDock restarts; running containers stay running. Finish Docker or Compose
operations before restarting. Downloads are checked against `SHA256SUMS` from
the same GitHub release; this version does not use publisher signatures.
The Debian package and portable archive offer the release-page link in Settings
and are updated manually.

Alternatively, extract
`ferrydock-0.15.0-linux-x86_64.tar.gz` and run `./ferrydock` from the extracted
directory. Its `INSTALL.txt` lists the runtime dependencies. Each release
includes a `SHA256SUMS` file to verify its downloads. On macOS, use
`shasum -a 256 <downloaded-file>` and compare the result with its entry in
`SHA256SUMS`.

To build from source instead, install Rust and the dependencies for
[Linux](docs/development.md#build-on-linux) or
[macOS](docs/development.md#build-on-macos), then run the commands below.
Windows build instructions are in the [development guide](docs/development.md#build-on-windows).

```sh
cargo build --release --locked
./target/release/ferrydock
```

On Linux, launch it from a terminal in your desktop session so it can use your
display and Docker environment.

### Your first container

1. Start Docker and launch FerryDock. The active Docker context appears in the
   window.
2. Find your container in its Compose project or the standalone group. Use the
   search field to narrow the list. Uncheck **Running only** to see
   stopped containers.
3. Select a container to open the detail panel. **Logs** shows its output;
   **Inspect** shows its configuration.
4. To run commands in a running container, choose **Terminal**, select **Auto**,
   **Bash**, or **sh**, and click **Open shell**. Auto uses Bash when available
   and falls back to sh.
5. Use **Start**, **Stop**, or **Restart** at the right of a container row, or in
   the detail panel for the selected container. **⋯** opens its resource menu.
   Drag the divider above the detail panel to give logs or the terminal more
   space.

Type `exit` to end a shell normally. Stopping or restarting a container closes
its terminal session; open a new shell after it starts again.

### Favorite commands

Click **Favorites** in a container or Compose project toolbar. For a project,
choose the service, then **Add command**. Enter a label and a command for your
chosen shell; multiline commands are supported. **Save favorite** stores it on
this computer without executing it. Use **Edit** or **Delete** beside a saved
command to update or remove it.

For quick access, press **Ctrl+K** or click **Run favorite…** in the app header.
Search by command, project, or service, then press **Enter**. FerryDock finds the
running target, opens an interactive quick-run console, and sends the command
once its shell is ready. Multiple running replicas get an instance picker;
stopped services stay stopped.

Use **Pin** beside a saved favorite to put a launch button on its Compose project
header (up to three buttons). Recent commands appear first in the launcher.
Quick runs use separate shells from the container's regular Terminal. Idle quick
shells are reused for the same target, keeping their directory and environment;
a busy shell stays intact and another session opens. **Hide** keeps commands
running; **Quick runs** in the app header returns to their output and result.
**End session** disconnects the selected shell. At most six quick shells stay open.

Open the container's **Terminal** and click **Open shell**. The favorites list
sits immediately above the terminal: select a command and click **Run**. The
command appears in the shell like typed input, and the shell stays open afterward.
**Running**, **Completed · exit 0**, or **Failed · exit …** makes the result
explicit. Output and the returned prompt remain visible; you can run another
favorite or continue typing normally. Changes of directory and exported variables
carry over to the next command in that shell.

The selected container is the execution target, including when a service has
several replicas. Select another container to use a different instance. **Manage…**
opens the add/edit/delete dialog; saving never runs a command.

**Run** is available only at an empty shell prompt. It stays disabled while a
command is running, while terminal input is unfinished, or while the container
is stopped or disconnected. Favorites never replace a session or start a service
automatically. Bash and sh report their prompt and exit status. Bash keeps your
normal startup settings and existing prompt hooks, including shell history.

Favorites belong to the Docker endpoint and Compose project/service names,
so they survive container recreation. Standalone favorites use the container
name. Changing endpoints keeps each endpoint's recipes separate. Recipes are
stored in `$XDG_CONFIG_HOME/ferrydock/favorites.json` (normally
`~/.config/ferrydock/favorites.json`); they are local plaintext, so avoid saving
secrets in commands. File errors appear in the dialog and unreadable files are
preserved.

### Manage a Compose project

Project headings show the Compose file path; hover it to see all files in order.
A **Git worktree** tag identifies a locally verified linked checkout containing
the primary Compose file. Paths unavailable on this computer cannot be verified.

FerryDock automatically associates accessible Compose files from Docker labels,
including labeled environment files. Click a project heading to open its controls
in the bottom dock. Use **Configuration** to review or change the detected files
and add profiles; saved configurations take precedence over automatic discovery.
If the labeled paths are missing or unavailable, choose **Associate files**.
Enter files in override order, one per line, and include the environment files
and profiles you normally use. Save the association to enable **Start**, **Stop**,
**Restart**, and **Rebuild & restart**. **Configuration** lets you edit it later.

Each project has a bordered group with indented container rows and a running/total
count. Click its chevron to collapse or expand the group; the standalone group
can be folded too. Folding keeps the current detail panel and session open.
Groups remember their folded state while FerryDock is open, including through
refreshes. Search temporarily reveals matching containers in folded groups;
clearing search restores the folds. Up/Down skips folded container rows.

**Stop** keeps containers, images and volume data so you can start the project
again. With **Running only** checked, stopped containers are hidden but their
project heading remains available.

Choose **More → Stop & cleanup…** to stop and remove a project, including stopped
containers and orphaned services. Review the containers and choose whether to
delete project volumes permanently and remove Compose-generated local images.
Both options are selected initially. External volumes and networks, bind-mounted
files, explicitly tagged images and build cache are kept. Successful cleanup
removes the saved association and the empty project heading. Progress and errors
appear in **Operations**; an error can leave a partially cleaned project.

The project's **Logs** tab combines output from all its containers, with a coloured
service/container prefix on each line that matches the container's list marker.
Lines are ordered by Docker's timestamps, so recent output stays at the bottom
even when another service's older history arrives later. The view retains the
newest 1,500 lines, starting with up to 200 lines per container.
Use the searchable service picker to choose **All services** or
one service, pause/resume following, and copy the
filtered retained output. Logs work directly from Docker labels without a file
association. The view picks up restarted or recreated services automatically.
**Operations** keeps Start/Stop/Restart output separate from application logs.
Source colours stay stable during the session, including after filtering or
recreating a container with the same name, and adapt to light and dark themes.
The twelve-colour palette cycles for larger lists. Application ANSI styles remain
scoped to each container. Displayed and copied logs preserve application-written
dates and omit Docker's additional timestamp prefix. Copied project logs keep
their source labels without ANSI escape sequences.

Start creates or starts project services without building images. Stop and
Restart operate on the configured project's services. **Rebuild & restart**
builds images using Docker's build cache, then recreates and starts the configured
services, including services that use an existing image. Build and startup output
appears in **Operations**. If a build fails, its error and output remain available
and the running services are left in place. The dock streams Compose
output and keeps the latest result, including failures, when you switch away
and return. Project groups remain accessible when their containers are stopped.
Conflicting container and project operations are blocked.

Associations are saved per Docker endpoint and project name in
`$XDG_CONFIG_HOME/ferrydock/compose.json`, or `~/.config/ferrydock/compose.json`.
Only file paths and profiles are saved; environment-file contents are not
copied. Saved projects remain listed even if no containers remain. Docker Compose
v2 must be installed, and the associated files must be available on this machine.

## Keyboard shortcuts

These shortcuts work when the container list has focus. In search, press
**Escape** to return to the list; **Ctrl+A** selects the search text so typing
replaces it, or Backspace clears it.

Tab focuses controls; Enter or Space activates a focused button. In Inspect,
Ctrl+F searches the JSON, Enter moves to the next match, and Escape closes search.

| Shortcut | Action |
| --- | --- |
| Ctrl+F | Focus search |
| Ctrl+R | Refresh containers |
| Ctrl+, | Open or close appearance settings |
| Up / Down | Select a container |
| Enter | Open a shell in the selected running container |
| Ctrl+1 | Open Logs |
| Ctrl+2 | Open Terminal |
| Ctrl+3 | Open Inspect |
| Ctrl+4 | Open the selected container's Compose project logs |

Inside the terminal, control keys go to the shell. Drag to select text, then use
**Ctrl+Shift+C** to copy. Paste with **Ctrl+Shift+V** or **Shift+Insert**, and
scroll with the mouse wheel or **Shift+PageUp / Shift+PageDown**.

## Manage images

Choose **Images** in the sidebar. The list refreshes automatically while this
workspace is open; **Refresh** or **Ctrl+R** reloads it manually. **Ctrl+F**
focuses image search, and Up/Down selects an image when the list has focus.
Each image appears once, with all its local tags in the detail panel. Untagged
images remain visible. Click **Image / ID**, **Age**, **Size (MiB)**, or **References** to
sort that column; click again to reverse the order. An arrow marks the active
direction. Sorting stays active through search and refreshes, and preserves the
selected image. Unavailable sizes and creation dates stay last in either direction.
Sizes use MiB and include shared layers, so they do not
represent the space that removal would recover. **Age** shows time since image
creation, such as **2 hours ago**, rather than when it was pulled to this computer.
Ascending age sorts youngest first; descending sorts oldest first. The detail
panel retains the exact creation date and time in UTC.

The detail panel shows the full image ID and running or stopped container
references in **Overview**. **History** shows Docker's build commands, newest
first, in a compact table: #, Command, Size (MiB), Created (UTC), and Layer ID.
Drag header dividers to resize columns; use **Columns** to hide metadata fields.
Widths and column choices stay active while you browse images. Long commands
stay inside their cells. Double-click a row, press Enter on a selected row, or
choose **View command…** from its right-click menu to read, search and select
the full instruction. **Ctrl+F** searches the table's commands. **Ctrl+C** or
**Copy command** copies the selected instruction without metadata, and the
right-click menu can also copy its full layer ID. Home/End select the first/last
command. All metadata columns start visible. Missing values show **—**,
while metadata-only instructions can have a zero size.
**Inspect** shows read-only, syntax-highlighted image JSON with selection,
folding, and Ctrl+F search, just like container Inspect. Ctrl+1, Ctrl+2, and
Ctrl+3 switch between Overview, History, and Inspect in the Images workspace.
**Analysis** (Ctrl+4) exports the selected local image and shows each build
instruction, added/replaced/deleted file paths, the largest final files, and
estimated waste from payloads replaced or removed in later layers. Choose
**Analyze image** to start, **Cancel** to stop, or **Reanalyze** to read it again.
Layer sizes and file payloads use MiB; metadata-only instructions are labelled.
Waste excludes archive overhead and duplicate-content scanning, and does not
measure the Docker disk space that removing the image would recover. Empty
images show efficiency as unavailable. Large images can reach an explicit
analysis limit. Switching image tabs retains the report; selection, refresh,
workspace closure, or disconnect clears it.
**Refresh** retries detail reads. Click a referenced container to open its existing container view.
**Ctrl-click** adds or removes an image from the selection; a normal click
selects only that image. Selected IDs stay selected through sorting, searches,
and refreshes. The selected count also identifies images hidden by your search.

**Remove image…** or **Remove selected (N)…** opens a confirmation popup listing every selected
image, including hidden selections, with **Cancel** and **Remove N images** controls.
Removal is unavailable while discovery is unavailable, an image/Compose operation
is in progress, or all selected images are in use. FerryDock checks a fresh
inventory, then asks Docker to remove each selected image without force or
automatic deletion of parent images. Images used by running or stopped containers
are kept. A failure does not stop the rest of the batch; a notification reports
the completion counts and up to three failure examples. Cancel or Escape closes
the popup without removing anything.
Successfully removed images leave the selection; failures stay selected while
they remain in Docker. Docker conflicts, including images with multiple tags,
leave those images in place.

**Prune…** opens a dialog with **Cancel**, **All untagged images**, and **All unused
images**. The first removes unused images without tags (Docker's dangling-image
prune); the second also removes tagged images unused by any container. Images
referenced by running or stopped containers are kept. Pruning applies to the
connected Docker daemon's full inventory, regardless of search or selection.
Removed images may need to be pulled or rebuilt again. The button shows progress,
then a notification reports completion and reclaimed space in MiB when Docker
provides it. The inventory refreshes afterward. Cancel or Escape closes the dialog
without pruning; conflicting Docker operations disable the action.
Pulling, tagging, and registry browsing are not available in this Images workspace.

## Manage volumes

Choose **Volumes** in the sidebar. Search names, drivers, or Compose project
labels; click a volume or use Up/Down when the list has focus. The list refreshes
automatically while this workspace is open. **Refresh** or **Ctrl+R** reloads it,
and **Ctrl+F** focuses search. The **Age** column shows elapsed time since creation
using the same relative wording as Images, from minutes through years. Missing
or invalid creation dates show **Unavailable**; future dates are shown explicitly. Click **Volume**, **Driver**, **Age**, **Size**, or
**References** to sort; click again to reverse direction. Age sorts by the actual
creation instant, and unavailable ages stay last in either direction. Sorting,
selection, and search survive refreshes and workspace changes.

**Ctrl-click** adds or removes volumes from the selection; a normal click selects
one. Selected volumes stay selected through sorting, search, and refreshes. The
selected count identifies any selections hidden by search.

The detail panel shows the driver, scope, creation date in UTC, mountpoint on
the Docker host, labels, driver options, and running or stopped containers using
the volume. Click a referenced container to open its container view. Stopped
containers become visible automatically. A Docker Desktop mountpoint belongs to
its Docker host and may be inside its VM.

Click **Gather volume sizes** to ask Docker to calculate volume disk usage. The
**Size** column and detail panel show MiB; click its header to sort by bytes in
either direction. Unmeasured or unsupported volumes show **Unavailable** and
stay last in either direction. Gathering runs in the background with loading and
error feedback. The last measurement time is shown beside the button.
Measurements stay cached during automatic refreshes, **Refresh**, **Ctrl+R**,
workspace changes, and reconnecting to the same Docker endpoint. Only another
click recalculates them; a failed calculation keeps the previous values. The
cache lasts for the app session. Removed/recreated volumes and a different
Docker endpoint discard their old measurements. Gathering requires Docker 23+
(API 1.42+) and can take longer for volumes with many files.

**Remove volume…** or **Remove selected (N)…** opens a confirmation listing every
selected volume, including hidden selections and volumes in use. Removal is
available after successful discovery, with at least one unused selected volume
and no conflicting Docker operation running. The confirmation explains that
removal permanently deletes stored data. **Cancel** or Escape closes it without
deletion. FerryDock checks fresh running and stopped container references, then
asks Docker to remove each selected volume without force. Referenced volumes
are kept; a failure does not stop the rest of the batch. A notification reports
removed/failed counts and up to three failure examples. Successful volumes leave
the selection; failed volumes stay selected while they remain in Docker.

**Prune…** opens a confirmation with two choices: **Anonymous volumes only**
removes unused volumes created without an explicit name; **All unused volumes**
also includes unused named volumes. Both cover the connected Docker daemon,
regardless of search or selection, and keep volumes referenced by running or
stopped containers. Removed volumes and their stored data are permanently deleted.
Cancel/Escape leaves Docker unchanged. While pruning runs, conflicting mutations
are disabled. Completion reports removed volumes and reclaimed MiB, refreshes
references, and retains cached sizes for surviving volumes. These prune modes
require Docker 23+ (API 1.42+).

Creating volumes and editing their configuration are not available.

## Explore networks

Choose **Networks** in the sidebar. Search by network name, ID, driver, Compose
project or subnet. Up/Down selects a network; **Ctrl+F** focuses search and
**Refresh** or **Ctrl+R** reloads the inventory and selected details. Both refresh
automatically every ten seconds while the workspace is open.

The list shows drivers, scopes, subnets in CIDR notation and configured container
counts, including stopped containers. Select a network to see its gateways,
internal/IPv6 settings and container endpoints. IPv4/IPv6 addresses include their
prefix lengths. Missing addresses and unreported DNS information show
**Unavailable**; an explicitly empty alias/DNS list shows **None reported**.
Stopped containers can retain configured aliases without an active address.
Endpoint read failures are reported on their row. **Copy** copies the full
container name, state, addresses, aliases/DNS names and any read error; hover it
to read values that do not fit the columns.

Click a container name to open its existing detail view. Containers absent from
the local inventory cannot be opened. In a container's **More** or right-click
menu, choose **View networks** to show only its configured networks and their
neighbours; **All networks** removes this filter. Ordinary network search retains
the selected detail. Switching workspaces preserves container/terminal sessions.

This workspace is read-only. It does not create/remove networks or connect/disconnect
containers. Host/none networks do not assign container IPs. Containers sharing
another container's network namespace and remote Swarm tasks may not appear as
local configured endpoints. Discovery and details are separate Docker reads,
so a changing attachment can briefly appear with an explicit detail error until
refresh. Local inventories are bounded; oversized inventories or slow detail
reads fail explicitly. The view does not test DNS resolution or connectivity.

## Docker connections

FerryDock uses `DOCKER_CONTEXT` if set, then `DOCKER_HOST`, then your active Docker
CLI context. Linux/macOS use local Unix sockets, including rootless Docker and
Docker Desktop contexts. Windows uses local Docker Desktop named pipes.
The context and endpoint are displayed in the app.
On macOS, Finder launches also search `~/.docker/bin`, Homebrew's bin directories,
and Docker Desktop's app bundle for the CLI and its credential helpers. The
keyboard shortcuts use Control on all platforms.

To launch with a particular local context:

```sh
DOCKER_CONTEXT=my-local-context ferrydock
```

If the Docker CLI is unavailable, FerryDock tries the default local Docker socket
or Windows named pipe. If the
daemon disconnects, FerryDock retries the connection automatically.

## Troubleshooting

| Problem | What to check |
| --- | --- |
| FerryDock cannot connect to Docker | Make sure Docker is running and your account can access it. If you have the Docker CLI, run `docker ps` from the same terminal. |
| Your context is unsupported | Select a local Unix socket context on Linux/macOS or a local named-pipe context on Windows. SSH and TCP/TLS connections are not supported. |
| A stopped container is missing | Uncheck **Running only** and clear the search field. |
| A shell will not open | The container must be running and include Bash or sh. Minimal images may have neither. |
| FerryDock reports a missing display | Launch from your Linux desktop session with Wayland or X11/XWayland available. |

## Current limits

Individual container controls and Compose project Start/Stop/Restart and
Rebuild & restart are available. Images and Volumes have their own workspaces.
A dedicated daemon-wide cleanup page is planned.

Container logs and combined project logs each retain up to 1,500 lines; project
streams begin with the last 200 lines per container and merge output as received.
Initial container log history larger than 32 MiB is rejected with an explicit
error to keep oversized records from exhausting application memory.
Terminal scrollback retains up to 5,000 lines.
The terminal does not yet support input method composition (IME) or mouse input
for terminal applications. **Disconnect** detaches FerryDock from a shell;
use `exit` when you want to end the shell process.

FerryDock uses native Wayland when `WAYLAND_DISPLAY` is set, or X11/XWayland
when only `DISPLAY` is set. Apple Silicon macOS and Windows x64 packages are
previews pending native desktop verification.

## Contributing

FerryDock is built with Rust and GPUI Kit. See the
[development guide](docs/development.md) for architecture, build instructions,
checks, verification coverage, and releases.

## License

FerryDock is available under the [MIT license](LICENSE).
