#!/usr/bin/env bash
# Build the GitHub Pages site into ./site for release TAG.
#
# Usage: build-site.sh <tag> <base-url> <artifacts-dir>
#
# Copies the release files found (recursively) in <artifacts-dir>, writes the
# `latest.json` manifest read by the in-app updater (src/updater/mod.rs) and
# renders the download page in every language (scripts/build-pages.py). The
# manifest's notes come from CHANGELOG.md (English, like the app's UI); GH_TOKEN and GITHUB_REPOSITORY are only needed when the version
# has no section there (the GitHub release text is used instead).
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

# Release notes come from CHANGELOG.md (the repository is private: users can't
# read it, so its sections are what the page and the update dialog show).
# Fall back to the GitHub release text for a version missing from it.
if ! notes=$(python3 scripts/changelog.py CHANGELOG.md "$TAG"); then
  notes=$(gh release view "$TAG" --repo "$GITHUB_REPOSITORY" --json body --jq .body)
fi

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
python3 scripts/build-pages.py site/latest.json "$dir" site

cat site/latest.json
