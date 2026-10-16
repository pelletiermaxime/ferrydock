# AppImage updates

Linux x86-64 AppImages check the latest stable GitHub release after startup and
every six hours. Settings → Updates offers automatic-check opt-out, manual check
and release notes. A verified download exposes a persistent Restart to update
banner and a review dialog with Later and Restart and apply. The application
never applies an update automatically. Other installation formats cannot
self-replace. SHA-256 checks use the release's SHA256SUMS; there are no publisher
signatures in this version.

Source: `src/updater.rs`, `src/app/updates.rs`, Settings composition in
`src/app/appearance.rs`, the banner in `src/app/shell.rs`, and the runtime
environment in `packaging/linux/AppRun`.

Commands:

```sh
cargo test --lib updater::tests -- --test-threads=1
python3 scripts/verify-headless.py --flow updates
```

The worker tests use owned temporary files and a loopback HTTP server. They cover
stable release selection, exact asset/checksum matching, bounded responses,
corrupted downloads, settings and retry intervals, verification immediately
before apply, installed-file changes, atomic replacement and launch-failure
rollback. They never touch the user's AppImage or Docker resources.

The focused GPUI flow wires a fixture updater channel before driving controls.
Inspect the Settings, downloading, ready, restart review, applying, failed
restart, current and non-AppImage screenshots. Assertions require the preference,
check and Apply requests, block duplicate checks, show that readiness and Later
do not apply, block new terminals while applying, and allow retry after failure.
No production update endpoint is overridden and no desktop application is exited.

Two ignored tests add explicit proof when invoked: `public_github_release_download_and_checksum`
uses the production HTTPS client to download and verify the current public
AppImage in owned files; `packaged_appimage_replacement_relaunch_and_startup_failure_rollback`
builds tiny disposable AppImages with the pinned cached tools and production
AppRun, verifies real extract-and-run replacement/relaunch through a path with
spaces, waits for its child to finish, and proves rollback on immediate failure.
The latter also runs in Linux release CI. See the development guide for commands.
Evidence stays under `.verification/appimage-updater/`.

These are separate UI, transport and runtime-fixture proofs. A published
FerryDock desktop old-to-new update, physical desktop input, and late startup
failure rollback remain outside this coverage. Six-hour scheduling is checked
through the preference/due calculation, not a six-hour wall-clock run.
