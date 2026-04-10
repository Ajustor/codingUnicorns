#!/usr/bin/env bash
#
# Installs the "cu" CLI alias for Coding Unicorns.
#
# Creates a symlink so you can open the IDE from any terminal:
#
#     cu .              # open current directory
#     cu path/to/folder # open a specific folder
#
# The script looks for coding-unicorns in:
#   1. ~/.cargo/bin  (cargo install)
#   2. /usr/local/bin
#   3. Anywhere in PATH
#
set -euo pipefail

# --- Locate the binary -------------------------------------------------------
BIN=""
for candidate in "$HOME/.cargo/bin/coding-unicorns" "/usr/local/bin/coding-unicorns"; do
  if [ -x "$candidate" ]; then
    BIN="$candidate"
    break
  fi
done

if [ -z "$BIN" ]; then
  BIN="$(command -v coding-unicorns 2>/dev/null || true)"
fi

if [ -z "$BIN" ]; then
  echo "Error: coding-unicorns not found. Install the app first (cargo install --path .)." >&2
  exit 1
fi

BIN_DIR="$(dirname "$BIN")"

# --- Create symlink -----------------------------------------------------------
LINK="$BIN_DIR/cu"
if [ -L "$LINK" ] || [ -e "$LINK" ]; then
  echo "Updating existing $LINK"
  rm -f "$LINK"
fi

ln -s "$BIN" "$LINK"
echo "Created symlink: $LINK -> $BIN"

# --- Verify PATH --------------------------------------------------------------
if command -v cu &>/dev/null; then
  echo "Done! You can now run:  cu <path>"
else
  echo ""
  echo "Warning: '$BIN_DIR' does not seem to be in your PATH."
  echo "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
  echo ""
  echo "    export PATH=\"$BIN_DIR:\$PATH\""
  echo ""
fi
