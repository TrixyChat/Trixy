#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This installer builder must run on macOS because it uses hdiutil." >&2
  exit 1
fi

for tool in cargo hdiutil ditto; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Required tool not found: $tool" >&2
    exit 1
  fi
done

rm -rf "$ROOT_DIR/dist"
bash "$ROOT_DIR/scripts/build-macos-app.sh"

APP_PATH="$ROOT_DIR/dist/Trixy.app"
if [[ ! -d "$APP_PATH" ]]; then
  echo "Expected app bundle was not created: $APP_PATH" >&2
  exit 1
fi

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n 1)"
if [[ -z "$VERSION" ]]; then
  VERSION="0.0.0"
fi
ARCH="$(uname -m)"
DMG_PATH="$ROOT_DIR/dist/Trixy-${VERSION}-${ARCH}.dmg"

STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/trixy-dmg.XXXXXX")"
cleanup() {
  rm -rf "$STAGING_DIR"
}
trap cleanup EXIT

# Conventional drag-to-Applications disk image.
ditto "$APP_PATH" "$STAGING_DIR/Trixy.app"
ln -s /Applications "$STAGING_DIR/Applications"

rm -f "$DMG_PATH"
hdiutil create \
  -volname "Trixy" \
  -srcfolder "$STAGING_DIR" \
  -ov \
  -format UDZO \
  "$DMG_PATH"

hdiutil verify "$DMG_PATH"

test -s "$DMG_PATH"
echo
echo "macOS installer build complete:"
echo "  $DMG_PATH"
