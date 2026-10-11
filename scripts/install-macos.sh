#!/usr/bin/env bash
#
# Install (or update) Coding Unicorns on macOS from the terminal:
#
#     curl -fsSL https://ajustor.github.io/codingUnicorns/install-macos.sh | bash
#
# Why a script: the app is signed ad hoc (no Apple Developer ID). A .dmg or
# .pkg downloaded by a browser gets the quarantine attribute, so Gatekeeper
# calls the app dangerous, and macOS 26 (Tahoe) even hangs it when it sits in
# a folder named "Applications". Files downloaded by curl carry no quarantine,
# and installing outside any "Applications" folder avoids the macOS 26 block.
#
# What it does:
#   1. reads the latest release from the download site (latest.json),
#   2. downloads the disk image and checks its SHA-256,
#   3. copies "Coding Unicorns.app" into ~/Apps (CU_INSTALL_DIR to change it),
#      where Spotlight and the Finder find it,
#   4. puts the `cu` command on the PATH, the way the app itself does
#      (src/cli_link.rs): /usr/local/bin when writable, else ~/.local/bin,
#      added to the shell's startup file when missing.
#
# Run it again to update; the in-app updater keeps it current afterwards.
set -euo pipefail

BASE_URL="${CU_BASE_URL:-https://ajustor.github.io/codingUnicorns}"
INSTALL_DIR="${CU_INSTALL_DIR:-$HOME/Apps}"
NAME="Coding Unicorns"
EXE="coding-unicorns"
ASSET="coding-unicorns-macos-arm64.dmg"
# Same marker as src/cli_link.rs, so the app recognises the line and never adds it twice.
PROFILE_MARKER='# Added by Coding Unicorns: the `cu` command'

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf 'Error: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || die "this script is for macOS."
[ "$(uname -m)" = arm64 ] || die "only Apple Silicon Macs (M1 and later) are supported."
case "$INSTALL_DIR" in
  */Applications | */Applications/*)
    die "macOS 26 blocks this app in any folder named \"Applications\": choose another CU_INSTALL_DIR." ;;
esac

work=$(mktemp -d)
mount=""
cleanup() {
  [ -n "$mount" ] && hdiutil detach -quiet "$mount" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

# --- Find the latest release -------------------------------------------------
say "Looking for the latest version"
curl -fsSL "$BASE_URL/latest.json" -o "$work/latest.json" ||
  die "could not reach $BASE_URL/latest.json"
# plutil reads JSON too: no python or jq needed on a fresh Mac.
field() { plutil -extract "$1" raw -o - "$work/latest.json" 2>/dev/null; }
version=$(field version) || die "latest.json has no version"
url="" sha=""
i=0
while name=$(field "assets.$i.name"); do
  if [ "$name" = "$ASSET" ]; then
    url=$(field "assets.$i.url")
    sha=$(field "assets.$i.sha256")
    break
  fi
  i=$((i + 1))
done
[ -n "$url" ] || die "version $version has no $ASSET"

# --- Download and check ------------------------------------------------------
say "Downloading Coding Unicorns $version"
curl -fL --progress-bar "$url" -o "$work/$ASSET"
actual=$(shasum -a 256 "$work/$ASSET" | cut -d' ' -f1)
[ "$actual" = "$sha" ] || die "checksum mismatch for $ASSET (expected $sha, got $actual)"

# --- Install the app ---------------------------------------------------------
if pgrep -x "$EXE" > /dev/null; then
  say "Coding Unicorns is running: quit it to use the new version"
fi
mount="$work/mnt"
mkdir "$mount"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mount" "$work/$ASSET"
app="$INSTALL_DIR/$NAME.app"
say "Installing into $app"
mkdir -p "$INSTALL_DIR"
rm -rf "$app"
ditto "$mount/$NAME.app" "$app"
hdiutil detach -quiet "$mount" && mount=""
# Nothing above sets it, but an older copy might have kept it.
xattr -dr com.apple.quarantine "$app" 2>/dev/null || true

# --- Put `cu` on the PATH ----------------------------------------------------
target="$app/Contents/MacOS/$EXE"
if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
  bin_dir=/usr/local/bin
else
  bin_dir="$HOME/.local/bin"
  mkdir -p "$bin_dir"
fi
link="$bin_dir/cu"
# Like src/cli_link.rs: a `cu` that isn't ours (a file, or a live link to
# another program) is never replaced.
ours=true
if [ -L "$link" ]; then
  current=$(readlink "$link")
  case "$(basename "$current")" in
    coding-unicorns*) ;;
    *) [ -e "$link" ] && ours=false ;;
  esac
elif [ -e "$link" ]; then
  ours=false
fi
if $ours; then
  ln -sfn "$target" "$link"
  say "Linked $link -> $target"
else
  say "$link belongs to another program: left as is (cu not installed)"
fi

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *)
    case "$(basename "${SHELL:-}")" in
      zsh) profile="$HOME/.zshrc" line="export PATH=\"$bin_dir:\$PATH\"" ;;
      bash) profile="$HOME/.bash_profile" line="export PATH=\"$bin_dir:\$PATH\"" ;;
      fish) profile="$HOME/.config/fish/conf.d/coding-unicorns.fish"
        line="fish_add_path --global \"$bin_dir\"" ;;
      *) profile="" line="" ;;
    esac
    if [ -z "$profile" ]; then
      say "Add $bin_dir to your PATH to use cu"
    elif ! grep -qxF "$line" "$profile" 2>/dev/null; then
      mkdir -p "$(dirname "$profile")"
      printf '\n%s\n%s\n' "$PROFILE_MARKER" "$line" >> "$profile"
      say "Added $bin_dir to the PATH in $profile (open a new terminal)"
    fi
    ;;
esac

# A copy dragged into /Applications from the .dmg hangs on macOS 26.
if [ -d "/Applications/$NAME.app" ]; then
  say "An older copy is still in /Applications: you can delete it"
fi

say "Done. Open Coding Unicorns from Spotlight, or run: cu ."
