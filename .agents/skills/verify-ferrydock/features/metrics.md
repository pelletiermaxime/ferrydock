# Container CPU and memory

Monitor running containers in the list and in the selected container detail dock.

## Sub-features

- `metrics.rows`: CPU percentage and memory used/configured limit in each container row.
- `metrics.summary`: CPU and memory values, native Kit area charts with labelled CPU-percent/MiB axes, crosshairs and measured-value hover tooltips above Logs, Terminal and Inspect.
- `metrics.collapse`: Hide graphs / Show graphs uses a Kit Collapsible and Button to switch between resource cards and a compact live CPU/memory/status summary. Expanded by default; the session-wide choice survives tab/container changes while sampling continues.
- `metrics.limits`: configured CPU quota and CPU set; memory limit versus explicitly labelled Docker capacity for unlimited containers.
- `metrics.freshness`: collecting, unavailable, stale, stopped and disconnected states; no fabricated zero for missing counters.

## User entry points

Launch FerryDock and read the running-container rows. Select a container to see its resource cards. Switch Logs, Terminal or Inspect; the cards remain above the tabs. Uncheck Running only to inspect a stopped container. A container detail dock has a 400-pixel minimum to keep its resource cards and tab content usable; project docks retain their 200-pixel minimum.

## Verification

`python3 scripts/verify-headless.py` feeds metrics through the production worker event boundary and drives selection and tabs through GPUI input. It captures collecting, unavailable, stale, stopped and unlimited states, asserts old-session/stopped sample rejection, checks painted CPU/memory chart and row pixels, and checks resource values above the terminal. Mouse movement exercises both chart tooltips and verifies changing CPU samples plus memory values from the actual byte counters. A separate 25 MiB / 2 GiB case proves readable low usage with an independent chart scale. Inspect `containers-and-logs`, `metrics-*`, `cpu-chart-hover`, `memory-chart-hover`, `metrics-low-usage-hover`, `terminal-rendered` and `kit-dock-resized` frames. This is real GPU rendering with fixture data, not live Docker metrics or physical desktop input.

`scripts/verify-docker.sh` owns a uniquely named Compose fixture with no published ports, host mounts or persistent volumes. Its probe has a 64 MiB memory limit and 0.5 CPU quota. The smoke test waits for real production-worker samples with a valid CPU baseline and asserts memory usage, effective capacity and configured limits. It also exercises existing lifecycle, logs and terminal checks. Retain its output and confirm scoped container/network cleanup; it does not drive the native UI.

Chart unit checks cover MiB values independent of the memory limit, adaptive scales and collection gaps/missing readings. Unit checks also cover multicore percentages above 100%, cgroup v1/v2 cache subtraction, CPU counter resets/missing baselines, legacy per-CPU count fallback, Docker-timestamp freshness, bounded history, automatic stale-state repaint and recovery. Native-window metrics, macOS, and daemon CPU/memory saturation remain outside this proof.

## Behavior

The general headless flow clicks Hide graphs, requires both chart cards to leave
the layout and the log viewport to gain over 120 logical pixels, switches to
Inspect and back while collapsed, then restores both charts without losing the
selected container, log text or retained samples. Inspect `metrics-collapsed-logs`,
`metrics-collapsed-inspect` and `metrics-expanded-again`. Restart persistence and
physical desktop collapse input are outside this check.

Sampling runs off the UI/command thread, with bounded request concurrency, timeouts and a two-second pause between rounds. Limits refresh every 30 seconds. CPU 100% represents one core; a quota is displayed separately. Unlimited memory uses Docker capacity for the percentage, with that denominator named explicitly. Readings older than eight seconds become stale. Plots fit the latest contiguous readings to their width, up to two minutes. The curves are visual interpolation; tooltips show actual measured samples. Memory uses a usage-based MiB scale, with configured limits and utilization kept in the footer. Stopped/removed containers lose their histories; reconnecting starts a new metrics session.
