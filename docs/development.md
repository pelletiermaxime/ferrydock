# Developing FerryDock

FerryDock is one Cargo package with separate UI, model, Docker worker, and
terminal modules. Docker I/O runs on a two-thread Tokio runtime away from the
GPUI thread. Event delivery is bounded, view updates are batched, and streams
are cancelled when selection changes or the window closes.

For the user guide, see the [README](../README.md).

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

FerryDock pins GPUI Kit to `0.7.3` with its `gpui-fast` feature enabled and all
direct GPUI engine/platform/WGPU packages to matching Fast version `0.1.7`. Its Linux
renderer uses wgpu and supports offscreen GPU screenshots for tests. The
[trial findings](gpui-kit-trial.md) record the compatibility changes.

## Headless UI verification

For the everyday UI development loop:

```sh
python3 scripts/verify-headless.py
```

Run verification commands from the repository root. This harness runs the real
FerryDock view and WGPU renderer with `DISPLAY` and `WAYLAND_DISPLAY` removed.
It dispatches GPUI keyboard and pointer events, asserts behavior, and retains
twenty-three GPU-rendered PNGs, input actions, frame timings, and a structured result
under `.verification/headless/<run>/`. No desktop window or Docker worker is
started. Fixture data enters through the Docker event channel; outgoing Stop
and terminal-input commands are checked at that boundary.

Headless PNGs use lossless adaptive filtering and balanced compression at full
resolution. The headless and native helpers automatically keep the newest eight
completed runs in their own default evidence folders. Active/incomplete runs,
failed resource cleanup, explicit `--evidence` paths, and runs with a `.keep`
file are preserved. Use `--keep-runs N` to change the limit or `--keep-runs 0`
to disable pruning. Historical links to automatically pruned runs may expire;
pin or export evidence that needs to remain available.

To preview or apply the same retention policy to existing headless/native runs:

```sh
python3 scripts/verification_evidence.py --dry-run
python3 scripts/verification_evidence.py
```

The flow covers discovery/grouping, search pointer focus, Select All, caret
navigation, selection replacement, Unicode deletion, empty matches/restored selection,
log-follow clicks, Stop requests, terminal ANSI output, and keyboard input
routing. It captures screenshots and checks painted glyphs/image changes. It
does not compare against golden images or prove compositor/clipboard behavior.

A Linux GPU/driver usable by WGPU is required. Renderer failure fails the run.
The optional `headless-tests` feature enables this harness; normal builds keep
test support disabled. The first build is slower; subsequent runs reuse it.
This verifies UI behavior and rendered output. The Docker integration check
below verifies actual container operations and shell execution.

## Code and Docker checks

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --lib --locked
bash scripts/verify-docker.sh
bash scripts/verify-compose.sh
```

The Docker verification script creates a uniquely named Compose project using
`alpine:3.23`, tests only that fixture, and removes its containers and network
on exit. It may download that image; the cached image remains. It does not prune
resources or operate on existing user containers. The integration test refuses
containers without both its fixture label and dedicated name prefix.

The integration check covers real Compose-label discovery, logs,
start/stop/restart, shell input, PTY resize, ANSI parsing, a full-screen `vi`
edit, shell exit, and a missing-Bash error. Unit tests cover split UTF-8,
bounded logs, replicas/filtering, terminal parser state, and terminal
input/output protocol.

The Compose check uses a separate uniquely named two-service project, ordered
base/override files, an environment file, and an explicit profile. It drives
the production worker commands for association and Start/Stop/Restart, checks
real Docker state and streamed success/failure output, and tests that ambient Compose variables
cannot change the saved invocation. Settings are isolated under its evidence
directory in `.verification/compose/`; containers and the network are removed
on exit. It does not drive a native desktop window.

The headless flow also drives the Compose association form, disabled controls,
project operation requests, and live/failure output through actual GPUI input.
Its fixture events do not execute Compose. Together these checks cover the UI
and real process boundary without requiring desktop automation for each change.

Project logs use Docker API streams for the selected project's labelled
containers, with membership updates from the existing snapshot watcher.
Service filters operate on a shared 1,500-line history. Per-container timestamp
cursors suppress replayed lines on stream retry; new container IDs get their
own stream and initial tail. The Compose integration fixture also proves mixed
stdout/stderr, resumed logs after restart and new membership after recreation.
Headless checks cover service-filter rendering, follow/pause, copy feedback,
recreated-member output and routing back to individual container logs. Native
clipboard behavior remains outside the headless proof.

Both log views use virtual lists that lay out only visible rows. The project
worker batches records for up to 50 ms, with at most 256 records and 64 KiB per
batch (a single larger bounded record is allowed). The UI drains events within
an 8 ms budget per poll. The headless flow fills the 1,500-line history, then
feeds a 10,000-line burst; `log-performance.json` records actual rendered rows,
follow position and input responsiveness. The Compose check independently
emits 10,000 real log records and verifies bounded batches without lost records;
the batch unit test separately proves record order and payload bounds.

`src/ansi.rs` uses the VTE processor already supplied by Alacritty to decode
output incrementally. Container logs and operation output retain a styled
buffer; project history keeps a decoder per container ID and caches each
completed record's text/style ranges. Source prefixes and Docker timestamps
stay neutral. Copy text contains no escape sequences. The raw transport
`model::LogBuffer` is unchanged so timestamp replay cursors still see the
original source records. Removed, expired source decoders are released during
snapshot reconciliation; quiet live containers retain their colour state.

Supported formatting includes standard/bright, indexed and RGB colours,
backgrounds, bold, italic, underline, strike, dim and inverse. Basic single-line
carriage-return, backspace and erase-in-line updates work in streamed container
and operation buffers. This is a record viewer, not a terminal screen: full-screen
cursor control and wide-character cursor editing are outside its contract.
Individual lines are bounded to 64 KiB, with at most 512 styled spans per line;
excess styling falls back to neutral text. Styling is parsed at ingestion,
and only visible cached rows are rendered, including Compose operation output.
Tests cover split escapes/UTF-8, resets, stream isolation, plain copy, progress
replacement, bounds and actual coloured pixels within GPU-rendered log rows.

For native log-load measurements, build the development binary and run:

```sh
cargo build --locked
python3 scripts/verify-log-performance.py --display :0
```

This owns an X11/XWayland window and ten disposable fixture containers producing
roughly 500 lines per second. It takes desktop focus, measures process/thread
CPU and RSS, checks search under load, retains screenshots/results under
`.verification/log-performance/`, and removes only its fixture and process.
Use `--binary target/release/ferrydock` after a release build for shipping
performance. Add `--ansi` to emit real coloured UTF-8 fixture records under the
same load. Capture timings bound when the visible result was obtained;
separate OCR time is verification overhead. Search capture time includes
synthetic typing at the established 20 ms cadence. It does not drive native
Wayland. GPUI and Taffy are optimized in the development
profile because unoptimized layout work can dominate live frames; FerryDock
itself stays unoptimized for fast rebuilds. `--profile` launches the owned child
under GDB and captures three UI-thread stack samples in `application.log`.

## Native UI verification

Recorded results and coverage limits are in
[VERIFICATION.md](../VERIFICATION.md).

The project-local
[verify-ferrydock skill](../.agents/skills/verify-ferrydock/SKILL.md) provides
the headless workflow, a native-window harness, an incremental
[feature map](../.agents/skills/verify-ferrydock/features/README.md), and
[execution evidence notes](../.agents/skills/verify-ferrydock/PROOF.md).
It lives in `.agents/skills/verify-ferrydock` for Codex discovery.

```sh
python3 .agents/skills/verify-ferrydock/scripts/native.py doctor --display :0
python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature terminal --display :0
```

The helper launches its own app and a disposable Compose fixture, verifies
selection before input, and retains screenshots/transcripts under
`.verification/ferrydock/` after scoped cleanup. It requires X11 desktop access,
`xdotool`, ImageMagick, Tesseract, and Python with Pillow. Use the actual display
on another workstation. Consult the feature map for exercised entry points and
mouse/display limitations; use `/maintain-verification-skill` as the app evolves.

## CI and releases

[Tests](../.github/workflows/ci.yml) runs on pushes to `main`, pull requests,
and manual dispatches. It checks formatting, Clippy, unit tests, the disposable
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
Native UI coverage is recorded separately above.

For a release, update the Cargo version and lockfile, add user-facing notes in
`docs/releases/v<version>.md`, commit the changes, and push the matching tag:

```sh
git tag -a v0.3.0 -m "FerryDock 0.3.0"
git push --atomic origin main v0.3.0
```

The tag must match the Cargo version and have a release-notes file. The publish
job is the only job with write access to repository contents.

To package a locally built release on an Ubuntu 24.04 x86-64 host, install
`dpkg-dev`, `curl`, `file`, Python 3, and the Linux build dependencies, then
build the binary and run:

```sh
bash scripts/package-linux.sh v0.2.0
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

FerryDock is [MIT licensed](../LICENSE). GPUI-CE, Bollard, and Alacritty's terminal
library retain their own Apache-2.0 licenses. No upstream sources are vendored
or patched.
