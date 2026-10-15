#!/usr/bin/env python3
"""Copy the static site, optionally refreshing downloads from a GitHub release."""

import argparse
import json
from pathlib import Path
import re
import shutil


ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", type=Path, help="GitHub releases/latest API JSON")
    parser.add_argument("--output", type=Path, default=ROOT / ".verification/website/dist")
    args = parser.parse_args()
    source = ROOT / "website"
    output = args.output.resolve()
    # Never let a typo overwrite repository source or its parent directories.
    if output == ROOT or output in ROOT.parents or output == source or source in output.parents:
        parser.error("output must be outside the website source and repository ancestors")
    release = json.loads((source / "release.json").read_text())
    if args.release:
        metadata = json.loads(args.release.read_text())
        patterns = {
            "appimage": r"ferrydock-[\d.]+-linux-x86_64\.AppImage",
            "deb": r"ferrydock_[\d.]+_amd64\.deb",
            "tarball": r"ferrydock-[\d.]+-linux-x86_64\.tar\.gz",
            "dmg": r"ferrydock-[\d.]+-macos-arm64\.dmg",
            "checksums": r"SHA256SUMS",
        }
        downloads = {}
        for kind, pattern in patterns.items():
            matches = [asset for asset in metadata["assets"] if re.fullmatch(pattern, asset["name"])]
            if len(matches) != 1:
                raise SystemExit(f"Expected one {kind} asset, found {len(matches)}; site was not published")
            downloads[kind] = matches[0]["browser_download_url"]
        release = {"version": metadata["tag_name"], "url": metadata["html_url"], "downloads": downloads}
    shutil.copytree(source, output, dirs_exist_ok=True)
    (output / "release.json").write_text(json.dumps(release, indent=2) + "\n")
    (output / ".nojekyll").touch()
    print(f"Static site: {output} ({release['version']}, {len(release['downloads'])} download links)")


if __name__ == "__main__":
    main()
