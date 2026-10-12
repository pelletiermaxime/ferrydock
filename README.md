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
- **Follow live logs.** Read timestamped output, pause automatic scrolling, copy
  logs, and reconnect the stream.
- **Open a terminal inside a container.** Use Bash or sh with colors, scrollback,
  text selection, copy and paste, and support for full-screen terminal programs.
- **Start, stop, and restart containers.** See progress and any errors in the
  detail panel.
- **Inspect container configuration.** View the full Docker inspect result as
  JSON.

## Get started

FerryDock is in early development. The current version supports **Linux with
X11 or XWayland** and a **local Docker daemon**. You need a working graphics
driver and permission to access Docker from your user account. FerryDock uses
your existing Docker installation.

### Install

For now, build FerryDock from source. Install Rust and the
[Linux build dependencies](docs/development.md#build-on-linux), then run these
commands from the FerryDock source directory:

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
   search field to narrow the list. Switch **Running only** to **All** to see
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

## Keyboard shortcuts

These shortcuts work when the container list has focus. In search, press
**Escape** to return to the list; **Ctrl+A** clears the search field.

| Shortcut | Action |
| --- | --- |
| Ctrl+F | Focus search |
| Ctrl+R | Refresh containers |
| Up / Down | Select a container |
| Enter | Open a shell in the selected running container |
| Ctrl+1 | Open Logs |
| Ctrl+2 | Open Terminal |
| Ctrl+3 | Open Inspect |

Inside the terminal, control keys go to the shell. Drag to select text, then use
**Ctrl+Shift+C** to copy. Paste with **Ctrl+Shift+V** or **Shift+Insert**, and
scroll with the mouse wheel or **Shift+PageUp / Shift+PageDown**.

## Docker connections

FerryDock uses `DOCKER_CONTEXT` if set, then `DOCKER_HOST`, then your active Docker
CLI context. Local Unix socket connections are supported, including rootless
Docker contexts. The context and endpoint are displayed in the app.

To launch with a particular local context:

```sh
DOCKER_CONTEXT=my-local-context ./target/release/ferrydock
```

If the Docker CLI is unavailable, FerryDock tries a local Docker socket. If the
daemon disconnects, FerryDock retries the connection automatically.

## Troubleshooting

| Problem | What to check |
| --- | --- |
| FerryDock cannot connect to Docker | Make sure Docker is running and your account can access it. If you have the Docker CLI, run `docker ps` from the same terminal. |
| Your context is unsupported | Select a local Unix socket context. SSH, TCP/TLS, and Windows named pipe connections are not supported yet. |
| A stopped container is missing | Switch **Running only** to **All** and clear the search field. |
| A shell will not open | The container must be running and include Bash or sh. Minimal images may have neither. |
| FerryDock reports a missing display | Launch from your Linux desktop session with X11 or XWayland available. |

## Current limits

This first version focuses on individual containers. Compose projects are
grouped for browsing; project-wide controls, rebuilding, and network or volume
management are planned. See the [roadmap](PLAN.md) for more.

Logs retain up to 1,500 lines; terminal scrollback retains up to 5,000 lines.
The terminal does not yet support input method composition (IME) or mouse input
for terminal applications. **Disconnect** detaches FerryDock from a shell;
use `exit` when you want to end the shell process.

Native Wayland, macOS, and Windows support are not part of this first Linux
version.

## Contributing

FerryDock is built with Rust and GPUI Community Edition. For build instructions,
checks, and UI verification, see the [development guide](docs/development.md).
Recorded verification results are available in
[VERIFICATION.md](VERIFICATION.md).

## License

FerryDock is available under the [MIT license](LICENSE).
