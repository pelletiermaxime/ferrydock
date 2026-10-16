# Favorite commands

Open **Favorites** in a container/project toolbar or **Manage…** above Terminal
for add/edit/delete. Projects offer a Kit service selector. Saving only persists
settings. Recipes use endpoint/project/service identity (standalone: name).

Open the selected container's Terminal and **Open shell**. Choose a favorite in
the Kit selector immediately above terminal output and click **Run**. The command
is sent to that existing shell; its echo, output and returned prompt remain
visible. **Running**, green **Completed · exit 0**, and red **Failed · exit N**
report its state. Repeated runs preserve shell cwd/exports. The selected container
is the explicit replica; favorites never choose another instance automatically.

Run is disabled without a connected empty shell prompt, during favorite/manual
commands, with unfinished input, and during Docker operation conflicts. The
worker rechecks session, endpoint, target and running state. Bash loads the normal
user startup file before installing prompt detection and preserves existing
scalar or array `PROMPT_COMMAND` hooks.

Container-operation conflicts apply to the favorite's target container. A restart
on another service or replica in the same Compose project leaves favorites
available. A Compose operation on the target's project, volume mutations and
image pruning still block launch.

Press **Ctrl+K** or **Run favorite…** for the Kit searchable launcher. Enter
runs the saved target directly when exactly one instance is running; otherwise
choose an instance or start the service yourself. The manager's **Pin** adds a
project-header button (first three pins). **Quick runs** reopens interactive
consoles; Hide keeps them alive and End session releases one. Idle quick shells
are reused, busy ones stay intact, and at most six are retained.

Run `python3 scripts/verify-headless.py --flow quick-run`. Inspect search,
connecting/running/completed/failed, project pin, replica picker and unavailable
frames. This exercises keyboard input from a regular terminal and quick console,
pin/recent-use persistence, independent sessions, idle reuse and stale target/
endpoint guards. The separate owned Docker test
`bash scripts/verify-docker.sh quick_run_terminals_preserve_interactive_sessions`
proves a blocked normal terminal and two quick PTYs retain independent input and
files, closing one preserves the others, and exit 0/1 reaches the right session.

The quick-run flow also checks launcher/pin protection during target and Compose
project restarts, then launches and sends a pending favorite while another service
in the same project restarts. The favorites flow checks the terminal Run button
blocks a target restart and permits another replica's restart. Inspect the
`quick-*-restart-*` and `favorites-*-restart-*` frames; these use worker events
for operation state and real GPUI input for launch.

Run `python3 scripts/verify-headless.py --flow favorites`. Inspect manager/editor,
terminal selector, running/completed/failed, partial-input protection, endpoint,
stopped and recreated frames. Require result passed, owned process exited and
all listed PNGs. Input uses the real GPUI view with a fixture worker and isolated
settings. It proves requests/rendering, not real shell or native desktop behavior.

Run `bash scripts/verify-docker.sh favorite_command_uses_an_interactive_pty`.
It owns a uniquely labelled Alpine fixture, checks PTY dimensions, interactive
input/output and an independently read proof file, consecutive same-shell runs,
exit 0/1, multiline cwd/export retention, and favorite/manual/partial-input guards.
Require PASS and scoped cleanup. The base image cache remains. The ignored test
also supports `FERRYDOCK_FAVORITE_SHELL=bash` or `auto` with a compatible owned
fixture image. Bash needs an image that contains it.
`FERRYDOCK_EXPECT_PROMPT_HOOK=1` additionally requires the fixture startup hook
to write `/tmp/ferrydock-existing-prompt-hook`, proving it survives integration.
Owned Ubuntu fixtures exercise scalar and array hooks; an owned fixture using
the Laravel development image also exercises its `www-data` startup settings.

Unit checks cover split prompt markers, unrelated/malformed escape preservation,
bounded decoding, multiline grouping, persistence CRUD/endpoint separation and
unreadable-file preservation. Native Favorites interaction, OS clipboard, other
startup configurations and macOS remain outside these checks. Preserve run
reports and screenshots under ignored `.verification/`.
