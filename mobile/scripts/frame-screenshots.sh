#!/bin/bash
# Frame screenshots with fastlane frameit and copy them into the metadata folders that the upload lanes read from.
#
# Usage:
#   ./scripts/frame-screenshots.sh [platform]
#
# Arguments:
#   platform  - "ios" or "android" (default: auto-detect)

set -euo pipefail

MOBILE_DIRECTORY="$(cd "$(dirname "$0")/.." && pwd)"
LOCALE="${LOCALE:-en-US}"

detect_platform() {
  if [ -n "${1:-}" ]; then
    echo "$1"
    return
  fi
  if xcrun simctl list devices booted 2>/dev/null | grep -q "Booted"; then
    echo "ios"
  elif adb devices 2>/dev/null | grep -q "device$"; then
    echo "android"
  else
    echo "android"
  fi
}

PLATFORM="$(detect_platform "${1:-}")"
SCREENSHOTS_DIRECTORY="$MOBILE_DIRECTORY/fastlane/screenshots/$PLATFORM/$LOCALE"

# "ipad" reuses the iOS frame config and uploads to the iOS metadata folder
# (App Store keeps every device size in one screenshots directory).
CONFIG_PLATFORM="$PLATFORM"
[ "$PLATFORM" = "ipad" ] && CONFIG_PLATFORM="ios"

if [ ! -d "$SCREENSHOTS_DIRECTORY" ]; then
  echo "Error: Screenshots directory not found: $SCREENSHOTS_DIRECTORY" >&2
  exit 1
fi

if ! command -v bundle &>/dev/null; then
  echo "Error: bundle not found. Install bundler and run 'bundle install' in mobile/." >&2
  exit 1
fi

echo "Preparing frameit configuration..."
cp "$MOBILE_DIRECTORY/fastlane-config/$CONFIG_PLATFORM/Framefile.json" "$SCREENSHOTS_DIRECTORY/"
cp "$MOBILE_DIRECTORY/fastlane-config/assets/background.jpg" "$SCREENSHOTS_DIRECTORY/"
cp "$MOBILE_DIRECTORY/fastlane-config/assets/Regular.ttf" "$SCREENSHOTS_DIRECTORY/"

cd "$SCREENSHOTS_DIRECTORY"

echo "Framing screenshots in $SCREENSHOTS_DIRECTORY..."
# frameit reads Framefile.json from the current directory; bundler walks up to mobile/Gemfile.
BUNDLE_GEMFILE="$MOBILE_DIRECTORY/Gemfile" bundle exec fastlane frameit silver

if [ "$PLATFORM" = "android" ]; then
  METADATA_DIRECTORY="$MOBILE_DIRECTORY/fastlane/metadata/android/$LOCALE/images/phoneScreenshots"
else
  METADATA_DIRECTORY="$MOBILE_DIRECTORY/fastlane/metadata/ios/$LOCALE/screenshots"
fi # iphone + ipad both land in the iOS screenshots dir

mkdir -p "$METADATA_DIRECTORY"

echo "Copying framed screenshots to metadata..."
# Pattern: [Device-Name-]NN_name_framed.png -> [Device-Name-]N.png
for source_file in *_framed.png; do
  [ -f "$source_file" ] || continue
  if [[ "$source_file" =~ ^(.*)-([0-9][0-9])_.*_framed\.png$ ]]; then
    device_prefix="${BASH_REMATCH[1]}"
    number_text="${BASH_REMATCH[2]}"
  elif [[ "$source_file" =~ ^([0-9][0-9])_.*_framed\.png$ ]]; then
    device_prefix=""
    number_text="${BASH_REMATCH[1]}"
  else
    echo "  Skipping unknown filename format: $source_file"
    continue
  fi

  number="$((10#$number_text))"
  if [ -n "$device_prefix" ]; then
    dest_name="${device_prefix}-${number}.png"
  else
    dest_name="${number}.png"
  fi

  cp "$source_file" "$METADATA_DIRECTORY/$dest_name"
  echo "  $source_file -> $METADATA_DIRECTORY/$dest_name"
done

echo "Done!"
