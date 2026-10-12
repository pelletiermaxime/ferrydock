#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo "Linux packaging requires an x86-64 Linux host" >&2
  exit 1
fi

version="$(python3 - <<'PY'
import pathlib
import tomllib
print(tomllib.loads(pathlib.Path("Cargo.toml").read_text())["package"]["version"])
PY
)"
if [[ $# -gt 0 && "$1" != "v$version" ]]; then
  echo "Tag $1 does not match Cargo version $version" >&2
  exit 1
fi

test -x target/release/ferrydock
command -v dpkg-deb >/dev/null
command -v dpkg-shlibdeps >/dev/null

output="$PWD/target/packages"
stage="$(mktemp -d "$PWD/target/package-stage.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$output"
archive="ferrydock-$version-linux-x86_64"
mkdir -p "$stage/$archive"
install -m755 target/release/ferrydock "$stage/$archive/ferrydock"
install -m644 LICENSE "$stage/$archive/"
sed "s/@VERSION@/$version/g" packaging/linux/INSTALL.txt >"$stage/$archive/INSTALL.txt"
install -m644 packaging/linux/dev.ferrydock.FerryDock.desktop "$stage/$archive/"
tar -czf "$output/$archive.tar.gz" -C "$stage" "$archive"

deb="$stage/debian/ferrydock"
mkdir -p "$deb/DEBIAN" "$deb/usr/bin" "$deb/usr/share/applications" \
  "$deb/usr/share/doc/ferrydock" "$stage/debian"
install -m755 target/release/ferrydock "$deb/usr/bin/ferrydock"
install -m644 packaging/linux/dev.ferrydock.FerryDock.desktop "$deb/usr/share/applications/"
install -m644 LICENSE "$deb/usr/share/doc/ferrydock/copyright"
install -m644 "$stage/$archive/INSTALL.txt" "$deb/usr/share/doc/ferrydock/INSTALL.txt"

# dpkg-shlibdeps derives library/version requirements from the built ELF binary.
cat >"$stage/debian/control" <<'CONTROL'
Source: ferrydock
Section: utils
Priority: optional
Maintainer: Maxime Pelletier <pelletiermaxime@users.noreply.github.com>

Package: ferrydock
Architecture: amd64
Description: Native desktop app for managing Docker containers
CONTROL
dependencies="$(cd "$stage" && dpkg-shlibdeps -O -e debian/ferrydock/usr/bin/ferrydock)"
dependencies="${dependencies#shlibs:Depends=}"
test -n "$dependencies"
installed_size="$(du -sk "$deb/usr" | cut -f1)"
cat >"$deb/DEBIAN/control" <<CONTROL
Package: ferrydock
Version: $version
Architecture: amd64
Section: utils
Priority: optional
Maintainer: Maxime Pelletier <pelletiermaxime@users.noreply.github.com>
Homepage: https://github.com/pelletiermaxime/ferrydock
Installed-Size: $installed_size
Depends: $dependencies, libvulkan1, libegl1, libwayland-client0, libwayland-cursor0, libwayland-egl1
Recommends: fonts-dejavu-core
Description: Native desktop app for managing Docker containers
 Browse Compose projects, follow live logs, open container terminals,
 and start, stop, or restart individual containers from a Linux desktop.
 Requires Wayland or X11/XWayland, a working graphics driver, and a local Docker daemon.
CONTROL
dpkg-deb --root-owner-group --build "$deb" "$output/ferrydock_${version}_amd64.deb"
bash scripts/package-appimage.sh "$version" "$stage" "$output"
(
  cd "$output"
  sha256sum "$archive.tar.gz" "ferrydock_${version}_amd64.deb" "$archive.AppImage" >SHA256SUMS
  sha256sum --check SHA256SUMS
)
echo "Packages written to $output"
