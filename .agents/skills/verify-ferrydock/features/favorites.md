# Favorite commands

Open **Favorites** in a container or project toolbar. Projects offer a Kit
service selector; saved service names remain available if the service is absent.
Use **Add command**, label and multiline command fields, **Save favorite**,
**Edit**, and **Delete**. Saving never starts Docker execution.

**Run…** shows the exact command and a Kit instance selector. A single running
instance is selected; replicas require an explicit choice. **Run in terminal**
starts a new `sh -c` PTY only after fresh endpoint/identity/running-state checks.
Command text stays visible above terminal output. Existing terminal views,
including connecting or ended sessions, require **Disconnect** before another
favorite run. Stopped/absent targets and operation conflicts explain why Run is
disabled. No service starts automatically and no other replica is substituted.

Run `python3 scripts/verify-headless.py --flow favorites`. Inspect the editor,
saved list, replica choice, command preview/terminal, protected-terminal,
stopped/absent target and changed-endpoint PNGs. Require `result.json` passed,
`owned_process_exited: true`, and all `headless.json` frames. The flow uses actual
GPUI input with a fixture worker boundary and an isolated preferences path; it
does not execute real shell commands or touch a desktop window.

Run `bash scripts/verify-docker.sh favorite_command_uses_an_interactive_pty`
for the real Docker boundary. It owns a uniquely named labelled Alpine fixture,
checks PTY dimensions and interactive output, writes a proof file inside that
container, independently reads it, and verifies a second command cannot replace
the occupied terminal. Require exit 0, the explicit PASS line and scoped Compose
cleanup. The base image cache remains. This does not prove native Favorites UI
interaction, clipboard behavior or macOS.

Persistence unit checks cover save/reload/edit/delete, endpoint separation,
multiline command preservation, control-character rejection and unreadable-file
preservation. Favorites survive service container recreation by project/service
identity; standalone recipes use the container name. Files are private local
plaintext under the same config directory as Compose settings.
