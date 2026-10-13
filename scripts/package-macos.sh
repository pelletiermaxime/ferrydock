#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  echo "macOS packaging requires an Apple Silicon Mac" >&2
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

minimum_macos="${MACOSX_DEPLOYMENT_TARGET:-15.0}"
binary="$PWD/target/aarch64-apple-darwin/release/ferrydock"
test -x "$binary"
test "$(lipo -archs "$binary")" = arm64
# Bundling only the executable is safe only when dependencies are system libraries.
libraries="$(otool -L "$binary")"
if printf '%s\n' "$libraries" | tail -n +2 | awk '{print $1}' | \
    grep -Ev '^(/System/Library/|/usr/lib/)'; then
  echo "Release binary has a non-system library dependency; bundle it before shipping" >&2
  exit 1
fi

output="$PWD/target/packages"
stage="$(mktemp -d "$PWD/target/macos-package-stage.XXXXXX")"
mount="$stage/mounted"
cleanup() {
  if [[ -d "$mount/FerryDock.app" ]]; then
    hdiutil detach "$mount" || return 1
  fi
  rm -rf "$stage"
}
trap cleanup EXIT
app="$stage/image/FerryDock.app"
mkdir -p "$output" "$app/Contents/MacOS" "$app/Contents/Resources"
install -m755 "$binary" "$app/Contents/MacOS/ferrydock"
install -m644 packaging/macos/FerryDock.icns "$app/Contents/Resources/"
install -m644 LICENSE "$app/Contents/Resources/"
sed -e "s/@VERSION@/$version/g" -e "s/@MINIMUM_MACOS@/$minimum_macos/g" \
  packaging/macos/Info.plist >"$app/Contents/Info.plist"
sed -e "s/@VERSION@/$version/g" -e "s/@MINIMUM_MACOS@/$minimum_macos/g" \
  packaging/macos/INSTALL.txt >"$stage/image/INSTALL.txt"
ln -s /Applications "$stage/image/Applications"
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - --timestamp=none "$app"
codesign --verify --deep --strict --verbose=2 "$app"

dmg="ferrydock-$version-macos-arm64.dmg"
hdiutil create -volname "FerryDock $version" -srcfolder "$stage/image" \
  -format UDZO -ov "$output/$dmg"
codesign --force --sign - --timestamp=none "$output/$dmg"
codesign --verify --strict --verbose=2 "$output/$dmg"
hdiutil verify "$output/$dmg"
# Inspect the distributed filesystem, not just the staging directory.
mkdir "$mount"
hdiutil attach -readonly -nobrowse -mountpoint "$mount" "$output/$dmg"
codesign --verify --deep --strict --verbose=2 "$mount/FerryDock.app"
test "$(lipo -archs "$mount/FerryDock.app/Contents/MacOS/ferrydock")" = arm64
cmp "$app/Contents/MacOS/ferrydock" "$mount/FerryDock.app/Contents/MacOS/ferrydock"
test -L "$mount/Applications"
test -f "$mount/INSTALL.txt"
hdiutil detach "$mount"
(
  cd "$output"
  shasum -a 256 "$dmg" >SHA256SUMS
  shasum -a 256 --check SHA256SUMS
)
echo "Ad hoc signed Apple Silicon package written to $output/$dmg"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "dmg=$output/$dmg" >>"$GITHUB_OUTPUT"
fi
