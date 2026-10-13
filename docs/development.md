# Developing FerryDock

FerryDock is one Cargo package with separate UI, model, Docker worker, and
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
| `container_logs.rs` | Container log history, status, follow/scroll state, and rendering |
| `terminal.rs` | Embedded terminal session state and toolbar/view routing; the emulator remains in `src/terminal.rs` |
| `projects.rs` | Compose associations, editable forms, operation state, and project controls |
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

Identify Compose membership from project/service labels and scope saved
associations by Docker endpoint and project name. Labels and file suggestions
cannot reconstruct the original environment and profile choices. Persist
explicit associations so projects remain available without containers.

Run Compose v2 with structured arguments, preserving ordered files, environment
files, profiles, and the displayed endpoint. Stream output, retain failures,
and reject conflicting container/project operations. Cancelling or closing
terminates the tracked process; it does not roll back Docker effects.

For future Rebuild/Down and resource pages, keep service and project action
scope explicit. Standalone containers have no implied build recipe. Down must
preserve volumes by default; volume removal and volume cleanup require an
explicit choice. Account for references from stopped containers when presenting
volume removal, and reconcile daemon state after operations.

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

## Verification

Run checks from the repository root:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --lib --locked
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
| `bash scripts/verify-compose.sh` | Production worker/Compose CLI, saved configuration and override order, environment/profiles, lifecycle effects, streamed failures, restart/recreation log recovery, bounded batches |
| `python3 scripts/verify-log-performance.py --display :0` | Owned X11/XWayland window and ten disposable containers; CPU/RSS and search under live log load |

Docker checks create unique Compose fixtures on the existing daemon and remove
only their containers and networks. They may pull `alpine:3.23`; the image cache
remains. Compose settings are isolated under the evidence directory. Fixtures
have no host mounts, published ports, or persistent volumes.

Build with `cargo build --locked` before the native performance helper; use
`--binary target/release/ferrydock` after a release build to measure shipping
performance. `--ansi` emits coloured UTF-8 records; `--profile` captures UI-thread
stacks through GDB. The helper takes desktop focus and must not overlap another
native-input run. Measurements depend on host, build profile, and workload;
capture/OCR overhead is reported separately.

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
actual display on your workstation. See [VERIFICATION.md](../VERIFICATION.md)
for recorded results and the native paths still awaiting proof.

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

## CI and releases

[Tests](../.github/workflows/ci.yml) runs on pushes to `main`, pull requests,
and manual dispatches. It checks formatting, Clippy, Rust and Python tests, the disposable
Docker and Compose integration fixtures, and the headless UI flow with Mesa's
software GPU driver. Headless and Compose evidence are retained as workflow
artifacts for seven days.

[Release](../.github/workflows/release.yml) runs when a `v*` tag is pushed. It
reuses the test workflow, then builds Linux x86-64 packages on Ubuntu 24.04:

- A `.tar.gz` archive containing the binary, license, install guide, and desktop
  launcher.
- An `.deb` package with library dependencies derived from the built binary.
- An `.AppImage` with ELF dependencies and runtime-loaded Wayland libraries,
  an icon/launcher, and bundled library copyright notices. glibc, fonts, and
  the Vulkan/EGL graphics driver stack remain host requirements.
- `SHA256SUMS` for all three packages.

The workflow installs the Debian package and checks both binaries in a clean
Ubuntu container before publishing the GitHub Release. This startup check
verifies library loading and the missing-display error; it does not exercise
the native desktop UI. A separate clean Ubuntu container launches the AppImage
with `--appimage-extract-and-run` on its own Xvfb display using Mesa software
Vulkan. It checks that application libraries are loaded from the bundle,
captures rendered UI, and verifies keyboard search input through OCR. The
AppImage check has no Docker socket and does not prove Docker operations,
native Wayland, FUSE mounting, or hardware GPU compatibility. Its screenshots,
logs, and result are retained in the `appimage-proof` workflow artifact.
Native UI coverage is recorded in [VERIFICATION.md](../VERIFICATION.md).

For a release, update the Cargo version and lockfile, add user-facing notes in
`docs/releases/v<version>.md`, commit the changes, and push the matching tag:

```sh
git tag -a v0.4.0 -m "FerryDock 0.4.0"
git push --atomic origin main v0.4.0
```

Replace the example version with the version being released. The tag must match
the Cargo version and have a release-notes file. The publish
job is the only job with write access to repository contents.

To package a locally built release on an Ubuntu 24.04 x86-64 host, install
`dpkg-dev`, `curl`, `file`, Python 3, and the Linux build dependencies, then
build the binary and run:

```sh
bash scripts/package-linux.sh v0.4.0
```

Output goes to `target/packages/`. Local builds on newer distributions may
require newer runtime libraries; publish the Ubuntu-built workflow artifacts.
The AppImage builder and runtime are pinned and SHA-256 checked by
`scripts/package-appimage.sh`, with downloads cached in `target/appimage-tools/`.
The bundler uses Debian package metadata to retain copyright notices, so run
packaging on Ubuntu/Debian.

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

## Dependency licenses

FerryDock is [MIT licensed](../LICENSE). Dependencies retain their own licenses;
package manifests and bundled library copyright notices are authoritative.
No upstream sources are vendored or patched.
