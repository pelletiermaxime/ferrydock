# FerryDock

A native desktop app for managing your Docker containers on Linux.

Find a container, follow its logs, open a shell, and start or stop it from one
window. FerryDock groups containers by Docker Compose project and keeps logs,
the terminal, and container details close at hand.

![FerryDock showing a Compose project and an interactive container terminal](docs/screenshots/ferrydock-terminal.png)

## What you can do

- **Find containers quickly.** Browse Compose projects and standalone containers,
  search by name, image, service, or project, and switch between running and all
  containers.
- **Follow live logs.** Read timestamped output with ANSI colours and styles, pause automatic scrolling, copy
  logs, and reconnect the stream. Combine a Compose project's service logs in
  one view and filter them by service.
- **Open a terminal inside a container.** Use Bash or sh with colors, scrollback,
  text selection, copy and paste, and support for full-screen terminal programs.
- **Start, stop, and restart containers.** See progress and any errors in the
  detail panel. Associate Compose files to control a whole project and watch
  its operation output.
- **Inspect container configuration.** Read syntax-highlighted JSON with line
  numbers, text selection, folding, and built-in search. The view is read-only.
- **Adjust the workspace.** Drag the divider between the container list and
  detail panel. Action buttons show loading state, and notifications report
  completed operations and copy feedback.
- **Choose your theme.** Open **Settings** from the sidebar or toolbar to choose
  **FerryDock**, **Dark**, or **Light**. The original charcoal and mint FerryDock
  theme remains the default. Changes apply immediately and are remembered on
  this computer; **Use FerryDock theme** restores the default.
- **Use resource menus.** Right-click a container or project, or use **More** in
  its detail toolbar, for navigation, configuration, and available actions.
- **Navigate container logs.** Use visible scrollbars for history and long
  lines. Opening a container shows the newest retained output immediately,
  then follows new lines without replaying the initial history.

## Get started

FerryDock is in early development. The current version supports **Linux with
Wayland or X11/XWayland** and a **local Docker daemon**. You need a working graphics
driver and permission to access Docker from your user account. FerryDock uses
your existing Docker installation.

### Install

Download a package from [GitHub Releases](https://github.com/pelletiermaxime/ferrydock/releases/latest).
The release is for **Linux x86-64**, built on Ubuntu 24.04; it requires
glibc 2.39 or newer.

The examples below use `0.4.0`; replace it with the version you downloaded.
On Ubuntu 24.04 or a compatible Debian-based distribution, install the Debian
package:

```sh
sudo apt install ./ferrydock_0.4.0_amd64.deb
```

Launch **FerryDock** from your applications menu or run `ferrydock` from a desktop
terminal. The package installs its runtime library dependencies automatically.
You still need Docker and a working graphics driver.

For a portable download on a compatible distribution, use
`ferrydock-0.4.0-linux-x86_64.AppImage`:

```sh
chmod +x ferrydock-0.4.0-linux-x86_64.AppImage
./ferrydock-0.4.0-linux-x86_64.AppImage
```

The AppImage bundles application libraries and needs no package installation.
It still requires glibc 2.39 or newer, an installed font, your desktop's
Vulkan/EGL graphics driver stack, and Docker. If FUSE mounting is unavailable,
run it with `--appimage-extract-and-run`.

Alternatively, extract
`ferrydock-0.4.0-linux-x86_64.tar.gz` and run `./ferrydock` from the extracted
directory. Its `INSTALL.txt` lists the runtime dependencies. Each release
includes a `SHA256SUMS` file to verify all three downloads.

To build from source instead, install Rust and the
[Linux build dependencies](docs/development.md#build-on-linux), then run:

```sh
cargo build --release --locked
./target/release/ferrydock
```

Launch it from a terminal in your desktop session so it can use your display
and Docker environment.

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
5. Use **Start**, **Stop**, or **Restart** to control the selected container.
   Drag the divider above the detail panel to give logs or the terminal more
   space.

Type `exit` to end a shell normally. Stopping or restarting a container closes
its terminal session; open a new shell after it starts again.

### Manage a Compose project

Click a Compose project heading to open its controls in the bottom dock. Choose
**Associate files** to review the suggested project directory and Compose files.
Enter files in override order, one per line, and include the environment files
and profiles you normally use. Save the association to enable **Start**, **Stop**,
and **Restart**. **Configuration** lets you edit it later.

The project's **Logs** tab combines output from all its containers, with
timestamps and a service/container prefix on each line. Use the searchable service picker to choose **All services** or
one service, pause/resume following, and copy the
filtered retained output. Logs work directly from Docker labels without a file
association. The view picks up restarted or recreated services automatically.
**Operations** keeps Start/Stop/Restart output separate from application logs.
Colours remain scoped to each container. Copied logs contain readable text
with their source labels and timestamps, without ANSI escape sequences.

Start creates or starts project services without building images. Stop and
Restart operate on the configured project's services. The dock streams Compose
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

## Docker connections

FerryDock uses `DOCKER_CONTEXT` if set, then `DOCKER_HOST`, then your active Docker
CLI context. Local Unix socket connections are supported, including rootless
Docker contexts. The context and endpoint are displayed in the app.

To launch with a particular local context:

```sh
DOCKER_CONTEXT=my-local-context ferrydock
```

If the Docker CLI is unavailable, FerryDock tries a local Docker socket. If the
daemon disconnects, FerryDock retries the connection automatically.

## Troubleshooting

| Problem | What to check |
| --- | --- |
| FerryDock cannot connect to Docker | Make sure Docker is running and your account can access it. If you have the Docker CLI, run `docker ps` from the same terminal. |
| Your context is unsupported | Select a local Unix socket context. SSH, TCP/TLS, and Windows named pipe connections are not supported yet. |
| A stopped container is missing | Uncheck **Running only** and clear the search field. |
| A shell will not open | The container must be running and include Bash or sh. Minimal images may have neither. |
| FerryDock reports a missing display | Launch from your Linux desktop session with Wayland or X11/XWayland available. |

## Current limits

Individual container controls and Compose project Start/Stop/Restart are
available. Rebuild, Down, and network, volume, and cleanup pages are planned.

Container logs and combined project logs each retain up to 1,500 lines; project
streams begin with the last 200 lines per container and merge output as received.
Initial container log history larger than 32 MiB is rejected with an explicit
error to keep oversized records from exhausting application memory.
Terminal scrollback retains up to 5,000 lines.
The terminal does not yet support input method composition (IME) or mouse input
for terminal applications. **Disconnect** detaches FerryDock from a shell;
use `exit` when you want to end the shell process.

FerryDock uses native Wayland when `WAYLAND_DISPLAY` is set, or X11/XWayland
when only `DISPLAY` is set. macOS and Windows support are planned.

## Contributing

FerryDock is built with Rust and GPUI Kit. See the
[development guide](docs/development.md) for architecture, build instructions,
checks, and releases, and the [verification summary](VERIFICATION.md) for
recorded evidence and coverage limits.

## License

FerryDock is available under the [MIT license](LICENSE).
