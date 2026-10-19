# Embedded terminal

Open a real shell inside the selected running container, send keyboard input through FerryDock, see terminal output, and exit the session.

## Sub-features

- `terminal.open`: connect an interactive sh/Bash session.
- `terminal.input`: send text, Return and control/navigation keys to a PTY.
- `terminal.render`: render ANSI colors, cursor movement and alternate screens.
- `terminal.size`: send terminal dimensions and resize the PTY.
- `terminal.clipboard`: select/copy output and paste input, including bracketed paste.
- `terminal.exit`: close shell sessions and show exit/disconnect state.

## How to get to it (user POV)

- Select a running container, then press Enter from the main list to open its shell.
- Click Terminal or use Ctrl+2 outside terminal focus, choose Auto/Bash/sh, and click Open shell (Reopen shell after exit).
- Click the terminal body to focus it, then type. Use terminal navigation/control keys normally.
- Resize the window; use the mouse wheel or Shift+PageUp/Down for terminal history; drag to select. Ctrl+Shift+C/V copies/pastes with terminal focus; Shift+Insert and the Paste button also paste.
- Type `exit` and Return to leave the shell; click Disconnect to close only that attachment. Changing containers retains the shell, its output and favorite status; returning restores Terminal. Up to six container shells are retained, without evicting a live session. Ctrl+C stops a foreground command; closing an attachment alone does not prove command termination.

## Driving it with native.py

Preconditions: headless checks need cached dependencies and the GPUI Kit test runtime; native checks additionally need the skill's native Doctor and owned running Alpine probe. The native probe has sh, not Bash; its proof file path must be absent before typing.

- **Headless development check:** `python3 scripts/verify-headless.py` dispatches Enter, asserts a terminal request for the selected fixture, delivers ready/ANSI-output events through the channel, renders the terminal without screenshot readback, then types `echo verified` and Enter through GPUI. It asserts exact outgoing bytes `echo verified\r`. This proves terminal rendering/input routing; it does not execute a shell or prove Docker PTY transport.

- **Session navigation:** `python3 scripts/verify-headless.py --flow terminal` drives row/search/Open shell/Disconnect across seven fixture containers, checks the six-session limit, background output and pending sessions, hidden stop/removal/exit, slot recovery, generation reuse protection and Docker disconnect. `--flow favorites` also checks a running favorite across another container's shell, background completion and scoped Disconnect. These use fixture worker events, not real Docker execution.
- **Independent Docker PTYs:** `bash scripts/verify-docker.sh container_terminals_preserve_independent_sessions` opens regular shells in two owned containers, runs a foreground favorite in the first, opens/uses/disconnects the second, then completes the original through its existing PTY. Independent file reads prove container/input isolation; `stty size` proves resize routing. The peer has the script's project ownership labels so its cleanup trap removes it on failure too.

- **Open:** `python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature terminal --display :0` isolates/selects the fixture and presses Return. Proof: Terminal view and “Connected” in a native screenshot, after list/detail selection proof.
- **Type/observe:** the same command types a unique `printf … | tee /tmp/<project>.txt; stty size | tee /tmp/<project>-size.txt` shell command and presses Return. Proof: retained native screenshot and transcript, independent read-only `docker exec … cat` of the new file matching the unique marker, and positive PTY rows/columns in `side-effect-after.json`. Docker does not execute the product action.
- **Exit:** the same command types `exit` and Return. Proof: “Shell exited” appears in the native window before owned cleanup.
- **Other entry points:** the Terminal tab/Open shell/Reopen shell buttons, Auto/Bash/sh selector, explicit resize, clipboard/selection/Paste button, scrolling/Shift+PageUp/Down, alternate-screen applications, control keys and Disconnect/error states are source-backed paths. Extend the harness with real user input and corresponding proof; API integration tests alone do not establish native behavior.

## Gotchas

- Ctrl+1/2/3 are application shortcuts only outside terminal focus. Inside, those keys must reach the shell.
- The command text may appear because the shell echoes input. Only the independently observed newly created file proves execution.
- This basic flow observes initial PTY sizing; it does not prove a subsequent resize.
- Clipboard operations affect the user's desktop clipboard. Add deliberate fixture text and restore captured clipboard state when extending this flow.
- Never execute proof commands in an existing user container.
