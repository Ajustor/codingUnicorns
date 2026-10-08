#!/usr/bin/env bash
# Build the GitHub Pages site into ./site for release TAG.
#
# Usage: build-site.sh <tag> <base-url> <artifacts-dir>
#
# Copies the release files found (recursively) in <artifacts-dir>, writes the
# `latest.json` manifest read by the in-app updater (src/updater/mod.rs) and
# renders the download page from pages/index.html. Needs GH_TOKEN and
# GITHUB_REPOSITORY to read the release notes.
#
# Used by .github/workflows/release.yml (fresh build artifacts) and
# .github/workflows/pages.yml (assets downloaded from the latest release).
set -euo pipefail

TAG="$1"
BASE_URL="$2"
ARTIFACTS="$3"

dir="site/download/$TAG"
mkdir -p "$dir"
find "$ARTIFACTS" -type f ! -name '*.ico' -exec cp {} "$dir/" \;

notes=$(gh release view "$TAG" --repo "$GITHUB_REPOSITORY" --json body --jq .body)

assets='[]'
for f in "$dir"/*; do
  name=$(basename "$f")
  sha=$(sha256sum "$f" | cut -d' ' -f1)
  url="$BASE_URL/download/$TAG/$name"
  assets=$(jq --arg name "$name" --arg url "$url" --arg sha "$sha" \
    '. + [{name: $name, url: $url, sha256: $sha}]' <<<"$assets")
done

jq -n --arg version "${TAG#v}" --arg notes "$notes" --arg page "$BASE_URL/" \
  --argjson assets "$assets" \
  '{version: $version, notes: $notes, page_url: $page, assets: $assets}' \
  > site/latest.json

cp assets/icon.png site/icon.png
python3 scripts/build-pages.py pages/index.html site/latest.json "$dir" site/index.html

cat site/latest.json
