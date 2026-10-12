#!/usr/bin/env python3
"""Measure one Linux/X11 release launch, then close only that process."""
import argparse
import json
import os
import pathlib
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument("--binary", default="target/release/ferrydock")
parser.add_argument("--display", default=os.environ.get("DISPLAY", ":0"))
args = parser.parse_args()
binary = pathlib.Path(args.binary).resolve()
environment = dict(os.environ, DISPLAY=args.display)
environment.pop("WAYLAND_DISPLAY", None)
started = time.monotonic()
process = subprocess.Popen([str(binary)], env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

def cpu_ticks():
    fields = pathlib.Path(f"/proc/{process.pid}/stat").read_text().split(")", 1)[1].split()
    return int(fields[11]) + int(fields[12])

try:
    while time.monotonic() - started < 15:
        if process.poll() is not None:
            raise RuntimeError(process.stderr.read().decode(errors="replace"))
        found = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^FerryDock$"], env=environment, capture_output=True)
        if found.returncode == 0:
            visible_after = time.monotonic() - started
            break
        time.sleep(0.05)
    else:
        raise RuntimeError("No visible FerryDock window within 15 seconds")
    time.sleep(3)
    before = cpu_ticks()
    sample_started = time.monotonic()
    time.sleep(5)
    idle_cpu = (cpu_ticks() - before) / os.sysconf("SC_CLK_TCK") / (time.monotonic() - sample_started) * 100
    resident = int(pathlib.Path(f"/proc/{process.pid}/statm").read_text().split()[1]) * os.sysconf("SC_PAGE_SIZE")
    print(json.dumps({"binary_mib": round(binary.stat().st_size / 1048576, 2), "warm_launch_to_visible_seconds": round(visible_after, 3), "resident_mib": round(resident / 1048576, 1), "cpu_percent_over_5_seconds": round(idle_cpu, 2), "backend": "X11", "scope": "one warm launch; default Docker context and initial log view; GPU/platform allocations included"}, indent=2))
finally:
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
