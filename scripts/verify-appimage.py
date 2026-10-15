#!/usr/bin/env python3
"""Verify release AppImage startup and bundled libraries on an owned Xvfb display."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import time


def run(*args, env=None):
    return subprocess.check_output(args, env=env, text=True, stderr=subprocess.STDOUT, timeout=30)


def stop(process):
    if process is None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=10)


def verify(appimage, evidence):
    evidence.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("LD_LIBRARY_PATH", None)
    env.pop("DOCKER_CONTEXT", None)
    env["DOCKER_HOST"] = "unix:///tmp/ferrydock-appimage-no-daemon.sock"
    env["XDG_CONFIG_HOME"] = str(evidence / "config")
    env["XDG_CACHE_HOME"] = str(evidence / "cache")
    env["XDG_RUNTIME_DIR"] = str(evidence / "runtime")
    Path(env["XDG_CONFIG_HOME"]).mkdir()
    Path(env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
    app = display = None
    pid = None
    result = {"status": "failed", "appimage": str(appimage),
              "sha256": hashlib.sha256(appimage.read_bytes()).hexdigest()}
    try:
        # This must work before any display exists, including on a minimal
        # Ubuntu image without an installed .deb or application libraries.
        missing_display = subprocess.run(
            [str(appimage), "--appimage-extract-and-run"], env=env,
            text=True, capture_output=True, timeout=30,
        )
        (evidence / "missing-display.log").write_text(missing_display.stderr)
        assert missing_display.returncode == 1, missing_display.stderr
        assert "FerryDock needs a Wayland" in missing_display.stderr, missing_display.stderr
        result["missing_display_check"] = "passed"

        with (evidence / "xvfb.log").open("w") as log:
            display = subprocess.Popen(
                ["Xvfb", "-displayfd", "1", "-screen", "0", "1280x900x24", "-nolisten", "tcp"],
                stdout=subprocess.PIPE, stderr=log, text=True, start_new_session=True,
            )
            # Xvfb writes the chosen display only once it is ready.
            assert select.select([display.stdout], [], [], 15)[0], "Xvfb did not become ready"
            number = display.stdout.readline().strip()
            assert number.isdecimal(), "Xvfb failed to allocate a display"
            env["DISPLAY"] = f":{number}"
        with (evidence / "application.log").open("w") as log:
            app = subprocess.Popen([str(appimage), "--appimage-extract-and-run"], env=env,
                                   stdout=log, stderr=log, start_new_session=True)
        deadline = time.monotonic() + 45
        window = None
        while time.monotonic() < deadline:
            assert app.poll() is None, "AppImage exited before opening a window; see application.log"
            search = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "^FerryDock$"],
                                    env=env, text=True, capture_output=True, timeout=5)
            if search.returncode == 0:
                window = search.stdout.splitlines()[0]
                break
            time.sleep(0.2)
        assert window, "No visible FerryDock window"
        pid = int(run("xdotool", "getwindowpid", window, env=env).strip())
        assert os.getpgid(pid) == app.pid, "Window is not owned by this AppImage launch"
        executable = Path(f"/proc/{pid}/exe").resolve()
        assert executable.parts[-3:] == ("usr", "bin", "ferrydock"), str(executable)
        appdir = executable.parents[2]
        library_dir = appdir / "usr/lib"
        libraries = {str(path.relative_to(library_dir)): path
                     for path in sorted(library_dir.rglob("*.so*")) if path.is_file()}
        # linuxdeploy retains Debian package notices beside the application
        # license. Inspect those resources in the packaged artifact itself.
        assert (appdir / "usr/share/doc/ferrydock/LICENSE").is_file()
        copyright_files = sorted((appdir / "usr/share/doc").glob("*/copyright"))
        for package in ("libxcb1", "libxkbcommon0", "libxkbcommon-x11-0"):
            notice = appdir / "usr/share/doc" / package / "copyright"
            assert notice in copyright_files and notice.stat().st_size, f"Missing {package} notice"
        # Mesa drivers on newer desktops require newer Wayland symbols than
        # the release builder provides. They must use the host's libraries.
        for pattern in ("libwayland-*.so*", "libvulkan.so*", "libEGL.so*", "libGL*.so*", "libOpenGL.so*"):
            assert not list(library_dir.rglob(pattern)), f"AppImage bundles host library: {pattern}"
        library_env = env | {"LD_LIBRARY_PATH": str(appdir / "usr/lib")}
        for name in libraries:
            dependencies = run("ldd", str(appdir / "usr/lib" / name), env=library_env)
            assert "not found" not in dependencies, dependencies
        maps = Path(f"/proc/{pid}/maps").read_text()
        # Require the executable's real ELF dependencies to load from the
        # bundle. The Fast renderer uses Rust font discovery/rasterization on
        # Linux, so Fontconfig/FreeType are no longer linked dependencies.
        dependencies = run("ldd", str(executable), env=library_env)
        assert "not found" not in dependencies, dependencies
        required = {line.split()[0] for line in dependencies.splitlines() if " => " in line}
        assert {"libxcb.so.1", "libxkbcommon.so.0", "libxkbcommon-x11.so.0"} <= required
        assert {"libxcb.so.1", "libxkbcommon.so.0", "libxkbcommon-x11.so.0"} <= libraries.keys()
        for name in required & libraries.keys():
            library = libraries[name].resolve()
            assert str(library) in maps, f"{name} was not loaded from the bundle"
        (evidence / "bundled-libraries.json").write_text(json.dumps(sorted(libraries), indent=2) + "\n")
        (evidence / "bundled-copyrights.json").write_text(json.dumps(
            [str(path.relative_to(appdir)) for path in copyright_files], indent=2
        ) + "\n")
        (evidence / "loaded-libraries.txt").write_text(maps)
        result.update(window=window, pid=pid, executable=str(executable),
                      bundled_libraries=len(libraries), bundled_loading_check="passed",
                      bundled_copyrights=len(copyright_files), copyright_check="passed",
                      host_wayland_check="passed")

        assert app.poll() is None, "AppImage exited during startup checks; see application.log"
        result["window_startup_check"] = "passed"
        result["status"] = "passed"
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        # Let the AppImage runtime reap its child and remove the extracted
        # directory before falling back to process-group cleanup.
        if pid is not None:
            try:
                os.kill(pid, signal.SIGTERM)
                app.wait(timeout=10)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                pass
        stop(app)
        stop(display)
        result["owned_processes_exited"] = (
            all(p is None or p.poll() is not None for p in (app, display))
            and (pid is None or not Path(f"/proc/{pid}").exists())
        )
        if not result["owned_processes_exited"]:
            result["status"] = "failed"
            result["error"] = "Owned application/display cleanup failed"
        (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result))
        assert result["owned_processes_exited"], result["error"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("appimage", type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    verify(args.appimage.resolve(), args.evidence.resolve())
