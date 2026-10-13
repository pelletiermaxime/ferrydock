# FerryDock agent instructions

Keep FerryDock a small, fast native Docker workbench.

- **Check GPUI Kit first.** Before any UI work, inspect the components and
  customization options available in the pinned version. Prefer them over
  custom widgets or rendering, and briefly explain when they cannot fit.
- **Organize by feature.** Keep feature state, controls, and rendering together
  in focused modules. Keep `src/app.rs` for composition and shared coordination.
- **Protect responsiveness.** Keep Docker I/O off the UI thread, bound retained
  data, batch busy streams, and virtualize large lists.
- **Preserve the user experience.** Keep the existing default theme, consistent
  styling, and selection/session/focus behavior. Make data displays clear about
  units, scale, and unavailable values.
- **Verify proportionately.** Exercise changed behavior through user-facing
  input and inspect relevant screenshots. State what the checks actually prove;
  build success alone does not prove a working desktop interaction. Reuse warm
  builds and measure performance before claiming improvements.
- **Respect shared resources.** Use owned disposable verification fixtures and
  clean up only their resources. Preserve unrelated work in the checkout.
- **Keep documentation lean.** Keep the README user-focused, developer procedures
  in the development guide, and local reports/artifacts under ignored
  `.verification/`. Avoid duplicating those procedures here.

Read [README.md](README.md) for product behavior,
[docs/development.md](docs/development.md) for architecture and build/release
procedures, and the [verification skill](.agents/skills/verify-ferrydock/SKILL.md)
for testing workflows and coverage.
