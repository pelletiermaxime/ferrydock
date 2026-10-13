#!/usr/bin/env python3
"""Keep the newest completed headless/native evidence runs (eight by default)."""
import argparse
import fcntl
import json
from pathlib import Path
import re
import shutil

ROOT = Path(__file__).resolve().parents[1]
RUN_NAME = re.compile(r"\d{8}T\d{6}Z-[0-9a-f]{8}\Z")


def keep_runs(value):
    count = int(value)
    if count < 0:
        raise argparse.ArgumentTypeError("keep-runs must be nonnegative; 0 keeps all runs")
    return count


def completed(run, kind):
    """Only delete recognized runs whose owned resources finished cleanup."""
    if run.is_symlink() or not run.is_dir() or not RUN_NAME.fullmatch(run.name):
        return False
    if (run / ".keep").exists():
        return False
    try:
        result = run / "result.json"
        if result.is_symlink():
            return False
        report = json.loads(result.read_text())
        if report.get("status") not in ("passed", "failed"):
            return False
        # Explicit --evidence paths inside the default root are also preserved.
        if report.get("custom_evidence") or report.get("evidence") != str(run):
            return False
        if kind == "headless":
            return report.get("owned_process_exited") is True
        cleanup = run / "cleanup.json"
        return not cleanup.is_symlink() and json.loads(cleanup.read_text()).get("success") is True
    except (OSError, ValueError, AttributeError):
        return False


def prune_runs(root, kind, keep=8, current=None, dry_run=False):
    """Prune only this verifier's completed runs; leave everything else alone."""
    if keep < 0:
        raise ValueError("keep must be nonnegative")
    summary = {"keep_runs": keep, "removed": [], "errors": []}
    if keep == 0 or not root.exists():
        return summary
    if root.is_symlink():
        summary["errors"].append(f"Refusing symlink evidence root: {root}")
        return summary
    try:
        with (root / ".retention.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            runs = [run for run in root.iterdir() if completed(run, kind)]
            runs.sort(key=lambda run: run.name, reverse=True)
            # Retain the caller even if the clock moved backwards.
            retained = ([current] if current in runs else [])
            retained.extend(run for run in runs if run != current)
            for run in retained[keep:]:
                if not dry_run:
                    shutil.rmtree(run)
                summary["removed"].append(str(run))
    except OSError as error:
        summary["errors"].append(str(error))
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--keep-runs", type=keep_runs, default=8, help="Runs per verifier; 0 disables pruning")
    parser.add_argument("--kind", choices=("headless", "ferrydock"), action="append",
                        help="Default: both headless and native")
    parser.add_argument("--dry-run", action="store_true", help="List old runs without removing them")
    args = parser.parse_args()
    results = {kind: prune_runs(ROOT / ".verification" / kind, kind, args.keep_runs,
                                dry_run=args.dry_run)
               for kind in args.kind or ("headless", "ferrydock")}
    print(json.dumps({"dry_run": args.dry_run, "verifiers": results}, indent=2))
    return 1 if any(result["errors"] for result in results.values()) else 0


if __name__ == "__main__":
    raise SystemExit(main())
