#!/usr/bin/env bash
# Build "Coding Unicorns.pkg" — a macOS installer package.
#
# Usage: scripts/build-macos-pkg.sh <binary> <output.pkg>
#
# The installer requires admin rights (standard for a PKG). Its postinstall
# script (packaging/macos/pkg-scripts/postinstall) runs as root and:
#  - Strips com.apple.quarantine from the installed bundle (bypassing the
#    macOS 26 restriction that prevents xattr -d even without sudo for apps
#    downloaded from the internet).
#  - On macOS 26+ (Darwin 25+): moves the app to ~/Desktop because
#    /Applications is blocked for ad-hoc-signed apps on that release.
#  - On macOS 15 and earlier: leaves the app in /Applications as usual.
#
# The PKG itself is ad-hoc signed; the embedded .app is too.  Users may
# still see a Gatekeeper prompt for the PKG on first open, but can proceed
# via right-click > Open or System Settings > Privacy & Security.
set -euo pipefail

bin="$1"
out="$2"

NAME="Coding Unicorns"
EXE="coding-unicorns"
BUNDLE_ID="io.github.ajustor.codingunicorns"
version=$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)
version="${version%%-*}"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Build the .app bundle (same as the DMG script).
app="$work/root/Applications/$NAME.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 "$bin" "$app/Contents/MacOS/$EXE"
sed -e "s/@NAME@/$NAME/g" -e "s/@EXE@/$EXE/g" -e "s/@BUNDLE_ID@/$BUNDLE_ID/g" \
  -e "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
plutil -lint "$app/Contents/Info.plist"

iconset="$work/AppIcon.iconset"
mkdir "$iconset"
for size in 16 32 128 256; do
  sips -z "$size" "$size" assets/icon.png --out "$iconset/icon_${size}x${size}.png" > /dev/null
  double=$((size * 2))
  sips -z "$double" "$double" assets/icon.png --out "$iconset/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

# Prepare postinstall script.
scripts="$work/scripts"
mkdir "$scripts"
cp packaging/macos/pkg-scripts/postinstall "$scripts/postinstall"
chmod +x "$scripts/postinstall"

rm -f "$out"
pkgbuild \
  --root "$work/root" \
  --scripts "$scripts" \
  --identifier "$BUNDLE_ID" \
  --version "$version" \
  --install-location "/" \
  "$out"

echo "created: $out"
ls -l "$out"
