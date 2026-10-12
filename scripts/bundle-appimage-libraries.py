#!/usr/bin/env python3
"""Bundle ELF dependencies and GPUI's dlopen libraries, retaining license notices."""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess

# glibc and the graphics driver stack must match the host. Vulkan/EGL loaders
# and drivers are provided by the target desktop, not copied from the builder.
HOST_LIBRARIES = re.compile(
    r"^(ld-linux.*|lib(c|m|pthread|rt|dl|resolv|util)\.so\..*|"
    r"lib(EGL|GL|GLX|GLdispatch|OpenGL|vulkan)\.so\..*)$"
)
RUNTIME_LIBRARIES = (
    "libwayland-client.so.0",
    "libwayland-cursor.so.0",
    "libwayland-egl.so.1",
)


def output(*args):
    return subprocess.check_output(args, text=True)


def bundle(binary, appdir):
    libraries = appdir / "usr/lib"
    notices = appdir / "usr/share/doc/ferrydock/bundled-libraries"
    libraries.mkdir(parents=True)
    notices.mkdir(parents=True)
    cache = {}
    for line in output("ldconfig", "-p").splitlines():
        match = re.search(r"^\s*(\S+) .*\(.*x86-64.*\) => (\S+)$", line)
        if match:
            cache.setdefault(match[1], Path(match[2]))
    pending = [binary]
    for name in RUNTIME_LIBRARIES:
        if name not in cache:
            raise RuntimeError(f"Missing runtime library: {name}")
        pending.append(cache[name])
    bundled = {}
    scanned = set()
    while pending:
        path = pending.pop()
        if path in scanned:
            continue
        scanned.add(path)
        if path != binary and not HOST_LIBRARIES.fullmatch(path.name):
            name = path.name
            shutil.copy2(path, libraries / name, follow_symlinks=True)
            # Ubuntu/Debian library packages ship their attribution/license
            # texts here. Missing provenance is a packaging failure.
            owners = output("dpkg-query", "-S", str(path.resolve())).splitlines()
            package = owners[0].split(": ", 1)[0]
            copyright_file = Path("/usr/share/doc") / package.split(":")[0] / "copyright"
            shutil.copy2(copyright_file, notices / f"{package.replace(':', '_')}.copyright")
            bundled[name] = {"package": package, "source": str(path)}
        for line in output("ldd", str(path)).splitlines():
            if "not found" in line:
                raise RuntimeError(f"Unresolved dependency of {path}: {line.strip()}")
            match = re.match(r"\s*(\S+) => (/\S+) ", line)
            if match and not HOST_LIBRARIES.fullmatch(match[1]):
                dependency = Path(match[2])
                # Copy under the SONAME used by the executable, even when the
                # symlink target has a different filename.
                if dependency.name != match[1]:
                    raise RuntimeError(f"Unexpected library name: {line.strip()}")
                pending.append(dependency)
    (notices / "manifest.json").write_text(json.dumps(bundled, indent=2, sort_keys=True) + "\n")
    print(f"Bundled {len(bundled)} libraries with package copyright notices")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("appdir", type=Path)
    args = parser.parse_args()
    bundle(args.binary.resolve(), args.appdir.resolve())
