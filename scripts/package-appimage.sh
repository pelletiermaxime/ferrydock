#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version="$1"
stage="$2"
output="$3"
tools="$PWD/target/appimage-tools"
mkdir -p "$tools"

# Pin dependency deployment, the builder, and the embedded runtime. Verify
# cached downloads too, before executing any tool.
download() {
  local name="$1" url="$2" checksum="$3"
  if [[ ! -f "$tools/$name" ]]; then
    curl --fail --location --retry 3 "$url" -o "$tools/$name.tmp"
    mv "$tools/$name.tmp" "$tools/$name"
  fi
  printf '%s  %s\n' "$checksum" "$tools/$name" | sha256sum --check
}
download linuxdeploy-x86_64.AppImage \
  https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage \
  c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
download appimagetool-x86_64.AppImage \
  https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage \
  ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
download runtime-x86_64 \
  https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 \
  2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
for tool in linuxdeploy appimagetool; do
  chmod +x "$tools/$tool-x86_64.AppImage"
  mkdir "$stage/$tool"
  (
    cd "$stage/$tool"
    "$tools/$tool-x86_64.AppImage" --appimage-extract >/dev/null
  )
done

appdir="$stage/FerryDock.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/doc/ferrydock"
install -m755 target/release/ferrydock "$appdir/usr/bin/"
install -m644 LICENSE "$appdir/usr/share/doc/ferrydock/"
sed "s/@VERSION@/$version/g" packaging/linux/INSTALL.txt >"$appdir/usr/share/doc/ferrydock/INSTALL.txt"
sed 's/^Icon=.*/Icon=dev.ferrydock.FerryDock/' \
  packaging/linux/dev.ferrydock.FerryDock.desktop >"$stage/dev.ferrydock.FerryDock.desktop"
# Let linuxdeploy collect dependencies, apply its standard exclusions, patch
# their search paths, and install desktop assets. Graphics drivers and the
# Wayland libraries they link must come from the same host stack.
# Bundle libxcb explicitly despite linuxdeploy's default exclusion so the ELF
# can start (and explain a missing display) on a clean Ubuntu installation.
"$stage/linuxdeploy/squashfs-root/AppRun" \
  --appdir "$appdir" \
  --executable "$appdir/usr/bin/ferrydock" \
  --library /usr/lib/x86_64-linux-gnu/libxcb.so.1 \
  --desktop-file "$stage/dev.ferrydock.FerryDock.desktop" \
  --icon-file packaging/linux/dev.ferrydock.FerryDock.svg \
  --custom-apprun packaging/linux/AppRun \
  --exclude-library 'libvulkan.so*' \
  --exclude-library 'libEGL.so*' \
  --exclude-library 'libGL*.so*' \
  --exclude-library 'libOpenGL.so*' \
  --exclude-library 'libwayland-*.so*'
ARCH=x86_64 "$stage/appimagetool/squashfs-root/AppRun" \
  --runtime-file "$tools/runtime-x86_64" "$appdir" \
  "$output/ferrydock-$version-linux-x86_64.AppImage"
