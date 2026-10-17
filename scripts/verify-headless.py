#!/usr/bin/env python3
"""Run headless GPUI input tests without screenshots, a desktop or Docker."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import tomllib
import uuid

from verification_evidence import keep_runs, prune_runs

ROOT = Path(__file__).resolve().parents[1]


def resolved_gpui_version(manifest, lock):
    packages = {manifest["dependencies"][name]["package"]
                for name in ("gpui", "gpui_platform")}
    versions = set()
    for name in packages:
        resolved = {package["version"] for package in lock["package"]
                    if package["name"] == name}
        if len(resolved) != 1:
            raise RuntimeError(f"Expected one resolved version of {name} in Cargo.lock")
        versions.update(resolved)
    if len(versions) != 1:
        raise RuntimeError("GPUI packages must resolve to the same version")
    return next(iter(versions))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, help="New evidence directory; existing paths are refused")
    parser.add_argument("--flow", choices=("all", "volumes", "analysis", "networks", "favorites", "quick-run", "updates", "lifecycle"), default="all", help="Full application or focused workspace input/rendering flow")
    parser.add_argument("--timeout", type=int, default=600, help="Build plus test timeout in seconds")
    parser.add_argument("--keep-runs", type=keep_runs, default=8,
                        help="Completed default-path runs to retain; 0 keeps all")
    args = parser.parse_args()
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    evidence = (args.evidence or ROOT / ".verification/headless" / (stamp + "-" + uuid.uuid4().hex[:8])).resolve()
    evidence.mkdir(parents=True, mode=0o700, exist_ok=False)
    env = os.environ.copy()
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    env["FERRYDOCK_HEADLESS_EVIDENCE"] = str(evidence)
    test_name = {
        "all": "app::kit_tests::user_flow::journey",
        "volumes": "app::kit_tests::volumes::journey",
        "analysis": "app::kit_tests::image_analysis::journey",
        "networks": "app::kit_tests::networks::journey",
        "favorites": "app::kit_tests::favorites::journey",
        "quick-run": "app::kit_tests::quick_run::journey",
        "updates": "app::kit_tests::updates::journey",
        "lifecycle": "app::kit_tests::lifecycle::journey",
    }[args.flow]
    command = ["cargo", "test", "--lib", "--features", "headless-tests", "--locked", "--offline",
               test_name, "--", "--exact", "--nocapture", "--test-threads=1"]
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    report = {"status": "failed", "selected_flow": args.flow, "evidence": str(evidence), "command": command,
              "gpui_snapshot": resolved_gpui_version(manifest, lock),
              "gpui_engine": manifest["dependencies"]["gpui"]["package"],
              "framework": "gpui-kit", "real_gpu": False,
              "display": None, "wayland_display": None,
              "custom_evidence": args.evidence is not None,
              "cargo_lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest()}
    started = time.monotonic()
    error = None
    with (evidence / "test.log").open("w") as log:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log, stderr=log,
                                   start_new_session=True)
        try:
            code = process.wait(timeout=args.timeout)
            report["exit_code"] = code
            if code:
                raise RuntimeError("Headless UI test failed; see test.log")
            flow = json.loads((evidence / "headless.json").read_text())
            if flow["status"] != "passed" or not flow["checkpoints"]:
                raise RuntimeError("Missing successful headless flow report")
            report.update(status="passed", flow=flow)
        except (Exception, KeyboardInterrupt) as failure:
            error = str(failure) or type(failure).__name__
            if process.poll() is None:
                # Only the owned Cargo/test process group is terminated.
                import signal
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
        finally:
            report.update(wall_seconds=round(time.monotonic() - started, 3), error=error,
                          owned_process_exited=process.poll() is not None,
                          retained_png_count=len(list(evidence.glob("*.png"))))
            (evidence / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    summary = {key: report[key] for key in
               ("status", "evidence", "wall_seconds", "retained_png_count", "error")}
    if args.evidence is None:
        summary["retention"] = prune_runs(ROOT / ".verification/headless", "headless",
                                          args.keep_runs, current=evidence)
    print(json.dumps(summary, indent=2))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
