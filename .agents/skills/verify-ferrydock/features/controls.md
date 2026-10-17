# Container controls

Stop, start and restart a container through its row actions or detail-dock controls and watch its state update.

## Sub-features

- `controls.stop`: stop a running container.
- `controls.start`: start a stopped container.
- `controls.restart`: restart a container and observe a new start time.
- `controls.feedback`: show pending actions and daemon errors.

## How to get to it (user POV)

- Select a container row to open its detail dock.
- Each running row has Restart, Stop and a resource menu; stopped rows have Start and the menu. These actions target that row without selecting it. Another container's operation preserves the current detail/terminal session.
- Row actions remain visible. Narrow windows omit the ports column, then row metrics, to keep names and actions usable; the detail dock retains the full information.
- Uncheck Running only in the main header so stopped rows remain visible.
- Click Stop or Start in the dock, depending on current state.
- Click Restart in the dock. Watch its state/status and action feedback.

## Driving it with native.py

Preconditions: headless checks need cached dependencies and the GPUI Kit test runtime; native checks additionally need the skill's native Doctor, owned fixture selection and pointer calibration.

- **Headless development check:** `python3 scripts/verify-headless.py` clicks the painted detail Stop and row Start/Stop/Restart controls through GPUI and asserts the outgoing fixture container IDs/actions. Row menu lifecycle actions preserve selection, log generation, terminal identity and shell readiness. Pending clicks and project-operation conflicts are blocked. Default/Light, pending, menu and 820/1440-wide row checkpoints are recorded; the width checks also click Stop. This proves rendering and requests at the fixture worker boundary. No real container is changed; daemon lifecycle results and physical desktop input remain outside this flow.

The native fixture below can safely stop because it has no data or host mounts.

- **Keep stopped visible:** `python3 .agents/skills/verify-ferrydock/scripts/native.py run --feature controls --display :0` selects the probe and clicks Running only. Proof: All containers appears before any lifecycle action.
- **Stop/start:** the same command clicks Stop, waits for fixture daemon state `exited` and visible Start, then clicks Start and waits for `running` and visible Stop. Proof: timestamped native control inputs, screenshots, and read-only fixture state observation.
- **Restart:** the same command clicks Restart and waits for `running` with a changed `StartedAt`. Proof: before/after timestamps in `control-side-effects.json` and native screenshot.
- **Error/feedback paths:** failed daemon actions, pending feedback and other container states require separate fixture-specific native scenarios. The successful lifecycle flow does not prove them.

## Gotchas

- Stop changes a real Docker container. Require fixture ownership and selected-detail proof before clicking.
- Do not use Docker start/stop/restart as a substitute for these UI actions.
- Running only can remove a stopped row and change the selection; uncheck Running only first.
- The helper refuses clicks if focus changes or coordinate calibration cannot be observed.
