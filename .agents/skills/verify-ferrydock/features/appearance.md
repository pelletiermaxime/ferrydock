# Appearance and resource menus

Open Settings from the sidebar or toolbar, or use Ctrl+,. Choose FerryDock,
Dark or Light; FerryDock is the default charcoal/mint palette. Use FerryDock
theme restores it. The choice is saved separately from Compose associations.
Returning to Containers restores the active editor, form or terminal focus.
Right-click a container/project or use More in its detail toolbar for navigation,
copy commands (containers), configuration (projects), and available operations.
Uncheck Running only to include stopped resources. Project Logs has a searchable
service picker; stream collection continues while filtering.

Run `python3 scripts/verify-headless.py`. Checkpoints cover settings-ferrydock/light/dark,
containers-light/dark, settings-default-restored, terminal-light, container-action-
menu/context-menu and project-action-menu states. Require `result.json` passed,
owned teardown and the recorded UI checks. Theme changes are
made through real controls and assertions read state. The flow's appearance file
is isolated under its evidence directory; it never alters user settings.

Coverage includes pointer theme changes, reset, persisted JSON, Light log visibility,
terminal-session generation and input-focus continuity, Ctrl+, from a live
terminal, right-click Inspect navigation, blocked duplicate menu actions, the
Running only checkbox, and service-picker search/confirmation. Library tests
cover missing/malformed settings and ANSI hidden/inverted text using the actual
surface. Bright ANSI text gains readable contrast on Light; dark surfaces retain
their original colors.

This is headless GPUI Kit layout evidence at 1180x820 logical pixels, with a worker
fixture. It proves requests, layout and input effects. Screenshot/contrast checks,
native desktop, OS clipboard and real container/Compose execution require separate
proof. Success toasts can temporarily overlay upper-right
content. For broader viewport or OS checks, use the owned native fixture and
report the additional front-door actions that were actually driven.
