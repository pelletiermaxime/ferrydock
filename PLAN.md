# FerryDock

A small native Docker workbench for the everyday loop: find a service, read its logs, open a shell, restart it, or rebuild it without losing your place.

## Current implementation

The Linux app implements container browsing/search, Compose grouping, streaming
logs, an embedded terminal, inspect, and container Start/Stop/Restart. Native
Wayland and X11 are enabled. Compose projects now support saved configuration
associations, project Start/Stop/Restart, and streamed operation output in the
bottom dock. Start uses `up --detach --no-build`; Rebuild, Down, resource pages,
and cleanup remain planned. See `VERIFICATION.md` for actual proof boundaries.
The project Logs tab combines all labelled member streams, with service
filtering, follow/pause, copy, bounded history and restart/recreation discovery.
The container and log lists render only visible rows. Project log records are
batched before UI delivery; busy-history, burst and native-load checks cover
the performance boundary alongside the existing functional flows.

## Agreed direction

- Rust and GPUI.
- Linux first; keep platform-specific code isolated for later macOS and Windows support.
- An embedded terminal belongs in the first runnable version.
- Containers, Compose projects, networks, volumes, and cleanup are the core scope.
- Take workflow inspiration from Recomposable and broad interaction inspiration from Docker Desktop, Echo, and Podman Desktop. FerryDock's design and implementation are original. Echo source code has not been inspected or copied; research was limited to its public product README.

## Product shape

One main window. A narrow navigation sidebar, a compact resource list, and a detail workspace. Show connection state persistently. Prefer useful information density and keyboard access over decorative dashboards.

**Containers** is the default page. Group containers under their Compose project, with a separate group for standalone containers. Show running containers by default with an obvious All filter for stopped containers. Each row includes service/name, state and health, image, and published ports. Expand a project to see its services and replicas. Select a container to inspect it without navigating away from the list.

The detail workspace has **Logs**, **Terminal**, and **Inspect** tabs. A compact action bar offers Start/Stop, Restart, and a menu for other applicable operations. Compose services also offer Rebuild. A project offers Start, Stop, Restart, Rebuild, and Down as distinct actions.

The first design concept explores two arrangements: a bottom detail dock that leaves the resource list wide, and a side detail workspace that keeps a longer list visible. Both preserve the same resource and action model.

Podman Desktop is another product reference for resource navigation, connection status, and presenting Compose applications together in the container workflow. Use its product documentation to inform those interactions. Podman engine support is a possible later extension; it is not implied by using Podman Desktop as inspiration. Docker remains the first backend.

**Networks** shows drivers, scope, subnets, Compose ownership, and connected containers. Support inspection, simple network creation, and removal of eligible networks.

**Volumes** shows driver, Compose ownership, mount information, and container references, including references from stopped containers. Support inspection, simple named-volume creation, and explicit removal when unreferenced. Filesystem browsing can come later.

**Cleanup** exposes stopped containers, unused networks, dangling/unused images, and build cache as separate categories. Show what Docker can determine, estimates with clear limits, and the result of each operation. Volumes are a separate, explicitly selected cleanup action because unused data can still be valuable. Report actual reclaimed space after an operation. Candidate lists are advisory: state can change before execution, and prune endpoints can have broader eligibility than a list of explicitly selected resource IDs.

Keep image information and image cleanup accessible in v1. A dedicated Images page can follow once a real workflow calls for it.

## First usable milestone

A real Linux window connected to the user's Docker daemon, with:

1. A live container list, Compose grouping, selection, and connection/error states.
2. Streaming logs with follow/pause, timestamps, copy, and bounded history.
3. An embedded interactive shell with keyboard input, resize, scrollback, selection/copy, paste, and clean session teardown.
4. Start, stop, and restart actions with progress and visible failures.

Prove the whole loop on a small dedicated fixture: start a service, inspect its logs, enter its shell, resize it, stop it, and see the UI update. This establishes the native UI and terminal foundations before building the remaining pages.

## Compose discovery and management

Use Docker's canonical labels `com.docker.compose.project` and `com.docker.compose.service` to identify membership. Project identity includes the daemon/context; project names alone are insufficient across connections. Handle replicas, one-off containers, and removed/recreated container IDs.

Supplement labels with `docker compose ls --all --format json` and optional working-directory/config-file labels. These are discovery hints rather than a complete reconstruction of the original invocation. A project with no remaining containers cannot always be rediscovered from Docker state, so persist explicitly associated projects.

Resolve and preserve the ordered Compose files, project name, project directory, environment-file choices, profiles, and connection. Let the user associate missing files or configuration through a project settings dialog. Do not expose rebuild/up as ready until its required configuration is available. Container-level actions remain useful when project files are unavailable.

Delegate Compose behavior to the installed Docker Compose v2 CLI using structured arguments, never a shell command string. Keep the Docker API connection and Compose CLI endpoint aligned. Stream process output into an operation view, retain failures for inspection, and prevent conflicting operations on the same target.

A service Rebuild uses the associated project invocation plus `up -d --build --no-deps SERVICE`. A separate Recreate action can use `--force-recreate`; rebuilding and forcing recreation are different operations. For replicas, explain that Compose service operations may affect every replica. Project Rebuild can affect dependencies and multiple services and should show its scope.

Default Down preserves volumes. Removing project volumes is an explicit separate choice. A standalone container has no automatically recoverable build recipe, so it does not receive a misleading Rebuild button. Attaching a build recipe can be future work.

Cancellation should stop the tracked process and reconcile Docker state afterward; it must not imply rollback of work Docker has already performed.

## Architecture

Start with a single Cargo package and clear module boundaries. Split crates only when it improves iteration or independent testing.

| Module | Responsibility |
| --- | --- |
| `app` | GPUI window, actions, navigation, focus, theme, and resource views |
| `docker` | Docker API connection, snapshots, events, logs, resource actions, exec |
| `compose` | Project discovery, file association, invocation construction, streamed CLI operations |
| `terminal` | Terminal state, ANSI parsing, grid rendering, input encoding, selection, resize |
| `model` | Stable resource identities, project associations, operation and connection state |
| `settings` | Small local preferences and saved project/context associations |

Use **GPUI** for the native interface, **Tokio** for background I/O, and **Bollard** for the Docker Engine API. Keep network and process work away from the UI thread. Transfer typed updates into GPUI rather than sharing UI state with background workers.

Use the pinned GPUI Community Edition Git revision for the native interface and its wgpu Linux renderer. Both native Wayland and X11/XWayland are enabled. Evaluate component libraries only after checking their API, platform support, and license. Avoid a large dependency stack merely to assemble a sidebar and list.

Use **alacritty_terminal** as a candidate terminal-emulation core, with a FerryDock GPUI view rendering its cells. Confirm the integration in the first milestone. A text area displaying ANSI-stripped output is insufficient for interactive applications.

For a container terminal, request Docker exec with a TTY and attached input/output. Route input bytes to the exec stream, parse output into terminal state, and propagate view dimensions through Docker's exec resize API. Support Bash when installed and a selectable `sh` fallback; some images have no shell and need an actionable error. Disconnect a session cleanly when its container exits or is recreated. Keep terminal key handling distinct from global app shortcuts.

Initialize resource state with a snapshot, consume Docker events for updates, and reconcile after actions, stream failures, and reconnections. Batch/coalesce UI updates. Bound log and terminal history, cancel subscriptions when views close, and keep inactive resource polling minimal. Add metrics only where they aid a real decision.

Resolve the active local connection using Docker context/environment configuration, including rootless sockets. The first milestone can support local Unix-socket contexts and clearly identify unsupported endpoints. Design the abstraction so later TLS/SSH/named-pipe transports do not require rewriting views. Never silently perform a Compose operation against a different daemon from the one displayed.

## Delivery sequence

**A — Native foundation and daily loop.** Build the first usable milestone above. Validate shell behavior with an interactive application, not just `echo`. Measure release startup, idle memory, and binary size; record results without promising targets before measurement.

**B — Compose workflow.** Discover projects, associate their files, and implement project/service actions and streamed builds. Test ordered override files, missing directories, stopped projects, multiple replicas, and explicit environment/profile choices. Verify that rebuilding one selected service uses the intended scope.

**C — Resource pages.** Networks and volumes with ownership and references, creation and eligible removal, then categorized cleanup. Validate against disposable resources, including a stopped container referencing a volume. Verify explicit deletion choices and reclaimed-space reporting.

**D — Everyday polish.** Search/filter, keyboard navigation and a command palette, remembered layout, themes, reconnect behavior, release packaging, and performance improvements grounded in measurements. macOS and Windows follow the Linux workflow proof.

## Verification boundaries

- Unit-test Compose command construction, grouping/identity, configuration completeness, and stream/session state transitions where mistakes have material effects.
- Run Docker integration checks against uniquely named, disposable fixtures. Do not restart, rebuild, or prune the user's existing containers as part of routine verification.
- A passing build does not prove native interaction. Manually verify window rendering, focus, copy/paste, terminal input and resizing, and live updates on Linux and record that scope.
- Exercise Docker-unavailable and Compose-unavailable states, missing shell, failed build, daemon reconnect, and container removal during a shell/log session.
- Use release measurements to establish how quick and small FerryDock actually is. GPUI has GPU/platform dependencies; Rust alone does not guarantee a small binary or low idle footprint.

## Source notes

Research checked on 2026-10-12. This plan records the product direction; current implementation and validation are tracked above and in `VERIFICATION.md`.

- [Recomposable product README](https://github.com/janvandorth/recomposable): service status, rebuilds, workspace switching, logs, keyboard workflow.
- [Echo product README](https://github.com/echo-and/echo): product-level reference only. No source code inspected or reused.
- [Podman Desktop](https://podman-desktop.io/), [container workflow](https://podman-desktop.io/docs/containers), and [Compose workflow](https://podman-desktop.io/docs/compose): product references for navigation, container actions, and multi-container applications.
- [GPUI official site](https://www.gpui.rs/): native Rust UI framework and upstream documentation.
- [Docker Compose service labels](https://docs.docker.com/reference/compose-file/services/#labels): canonical project and service labels.
- [Docker Compose project listing](https://docs.docker.com/reference/cli/docker/compose/ls/): stopped-project and JSON options.
- [Docker Compose up](https://docs.docker.com/reference/cli/docker/compose/up/): build, dependency, recreation, and detached options.
- [Bollard exec API](https://docs.rs/bollard/latest/bollard/exec/index.html): Docker exec support.
- [Alacritty terminal library](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/): terminal-emulation core candidate.
- [Docker Engine API](https://docs.docker.com/reference/api/engine/): engine integration and versioning.
- [Docker system prune](https://docs.docker.com/reference/cli/docker/system/prune/): category semantics and anonymous-volume option.
