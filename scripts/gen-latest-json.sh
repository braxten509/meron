#!/usr/bin/env bash
# Build the update manifest (latest.json) from the release artifacts.
#
# The desktop updater reads this file from
# https://github.com/nonbili/meron/releases/latest/download/latest.json and matches on "<goos>-<goarch>"
# plus the install channel it detected.
#
# Only the self-updatable channels are listed: .snap and .appx are store-managed, so leaving them out is
# what makes those builds report "managed by your package manager" even if detection ever slips.
#
# Usage: scripts/gen-latest-json.sh <version> <asset-base-url> <dist-dir>
set -euo pipefail

version="${1:?version required}"
base_url="${2:?asset base url required}"
dist="${3:?dist dir required}"

# artifact filename -> "<platform> <channel>"; must stay in sync with the
# `artifact` names in .github/workflows/release.yml and the channel constants in
# update_channel.go.
entries=(
  "meron-darwin-arm64.dmg darwin-arm64 dmg"
  "meron-darwin-amd64.dmg darwin-amd64 dmg"
  "meron-linux-amd64.AppImage linux-amd64 appimage"
  "meron-linux-amd64.tar.gz linux-amd64 tarball"
  "meron-windows-amd64.exe windows-amd64 nsis"
  "meron-windows-amd64-portable.zip windows-amd64 portable"
)

args=()
filter='{version: $version, pubDate: $pubDate, platforms: {}}'
index=0
for entry in "${entries[@]}"; do
  read -r file platform channel <<<"$entry"
  path="$dist/$file"
  if [ ! -f "$path" ]; then
    echo "gen-latest-json: missing required artifact $path" >&2
    exit 1
  fi
  sha=$(sha256sum "$path" | cut -d' ' -f1)
  size=$(wc -c <"$path")
  args+=(--arg "url$index" "$base_url/$file")
  args+=(--arg "sha$index" "$sha")
  args+=(--argjson "size$index" "$size")
  filter+=" | .platforms[\"$platform\"][\"$channel\"] = {url: \$url$index, sha256: \$sha$index, size: \$size$index}"
  index=$((index + 1))
done

jq -n \
  --arg version "$version" \
  --arg pubDate "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "${args[@]}" \
  "$filter"
