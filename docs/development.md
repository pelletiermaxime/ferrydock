# Developing FerryDock

FerryDock is one Cargo package with separate UI, model, Docker worker, and
terminal modules. Docker I/O runs on a two-thread Tokio runtime away from the
GPUI thread. Event delivery is bounded, view updates are batched, and streams
are cancelled when selection changes or the window closes.

For the user guide, see the [README](../README.md).

## Build on Linux

Install Rust and the native build dependencies. Rust 1.99 was used for local
verification. On Debian/Ubuntu, the typical prerequisites are:

```sh
sudo apt install build-essential pkg-config cmake clang \
  libssl-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libfontconfig1-dev libfreetype-dev \
  libxcb1-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev
```

Dependencies were already available on the development machine; this package
command has not been verified on a clean Ubuntu installation.

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
The Linux app uses X11, including XWayland on a Wayland desktop. `DISPLAY` must
be set by the desktop session; no manual backend override is needed. Native
Wayland remains disabled until it can be verified with GPUI-CE.

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

## Dependency licenses

FerryDock is [MIT licensed](../LICENSE). GPUI-CE, Bollard, and Alacritty's terminal
library retain their own Apache-2.0 licenses. No upstream sources are vendored
or patched.
