#!/usr/bin/env python3
"""Run the real headless GPU UI flow, preserving evidence without a desktop or Docker."""
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, help="New evidence directory; existing paths are refused")
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
    command = ["cargo", "test", "--lib", "--features", "headless-tests", "--locked", "--offline",
               "headless_tests::headless_gpu_user_flow", "--", "--nocapture", "--test-threads=1"]
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    revisions = {manifest["dependencies"][name]["version"] for name in ("gpui", "gpui_platform", "gpui_wgpu")}
    if len(revisions) != 1:
        raise RuntimeError("GPUI packages must share one pinned snapshot version")
    report = {"status": "failed", "evidence": str(evidence), "command": command,
              "gpui_snapshot": next(iter(revisions)),
              "gpui_engine": manifest["dependencies"]["gpui"]["package"],
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
            if flow["status"] != "passed" or not flow["screenshots"]:
                raise RuntimeError("Missing successful GPU flow report")
            for name in flow["screenshots"]:
                if not (evidence / name).is_file():
                    raise RuntimeError("Missing retained screenshot: " + name)
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
