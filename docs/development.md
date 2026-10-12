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

All GPUI crates are pinned to Community Edition Git revision
`43dacc568497c615c55a1ea0f77ab381c0d9865b`. Its Linux renderer uses wgpu and
supports offscreen GPU screenshots for tests.

## Headless UI verification

For the everyday UI development loop:

```sh
python3 scripts/verify-headless.py
```

Run verification commands from the repository root. This harness runs the real
FerryDock view and WGPU renderer with `DISPLAY` and `WAYLAND_DISPLAY` removed.
It dispatches GPUI keyboard and pointer events, asserts behavior, and retains
six GPU-rendered PNGs, input actions, frame timings, and a structured result
under `.verification/headless/<run>/`. No desktop window or Docker worker is
started. Fixture data enters through the Docker event channel; outgoing Stop
and terminal-input commands are checked at that boundary.

The flow covers discovery/grouping, search/empty matches/restored selection,
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
cargo clippy --all-targets --locked -- -D warnings
cargo test --lib --locked
bash scripts/verify-docker.sh
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
Docker integration fixture, and the headless UI flow with Mesa's software GPU
driver. Headless evidence is retained as a workflow artifact for seven days.

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
git tag v0.1.1
git push origin v0.1.1
```

The tag must match the Cargo version and have a release-notes file. The publish
job is the only job with write access to repository contents.

To package a locally built release on an Ubuntu 24.04 x86-64 host, install
`dpkg-dev`, `curl`, `file`, Python 3, and the Linux build dependencies, then
build the binary and run:

```sh
bash scripts/package-linux.sh v0.1.1
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
