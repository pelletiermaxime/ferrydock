#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version="$1"
stage="$2"
output="$3"
tools="$PWD/target/appimage-tools"
mkdir -p "$tools"

# Pin both the builder and embedded runtime; appimagetool's default downloads
# a moving runtime. Verify cached downloads too, before executing either tool.
download() {
  local name="$1" url="$2" checksum="$3"
  if [[ ! -f "$tools/$name" ]]; then
    curl --fail --location --retry 3 "$url" -o "$tools/$name.tmp"
    mv "$tools/$name.tmp" "$tools/$name"
  fi
  printf '%s  %s\n' "$checksum" "$tools/$name" | sha256sum --check
}
download appimagetool-x86_64.AppImage \
  https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage \
  ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
download runtime-x86_64 \
  https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 \
  2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
chmod +x "$tools/appimagetool-x86_64.AppImage"
mkdir "$stage/appimagetool"
(
  cd "$stage/appimagetool"
  "$tools/appimagetool-x86_64.AppImage" --appimage-extract >/dev/null
)

appdir="$stage/FerryDock.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/doc/ferrydock"
install -m755 target/release/ferrydock "$appdir/usr/bin/"
install -m755 packaging/linux/AppRun "$appdir/AppRun"
install -m644 LICENSE "$appdir/usr/share/doc/ferrydock/"
sed "s/@VERSION@/$version/g" packaging/linux/INSTALL.txt >"$appdir/usr/share/doc/ferrydock/INSTALL.txt"
sed 's/^Icon=.*/Icon=dev.ferrydock.FerryDock/' \
  packaging/linux/dev.ferrydock.FerryDock.desktop >"$appdir/dev.ferrydock.FerryDock.desktop"
install -m644 packaging/linux/dev.ferrydock.FerryDock.svg "$appdir/"
ln -s dev.ferrydock.FerryDock.svg "$appdir/.DirIcon"
python3 scripts/bundle-appimage-libraries.py target/release/ferrydock "$appdir"
ARCH=x86_64 "$stage/appimagetool/squashfs-root/AppRun" \
  --runtime-file "$tools/runtime-x86_64" "$appdir" \
  "$output/ferrydock-$version-linux-x86_64.AppImage"
