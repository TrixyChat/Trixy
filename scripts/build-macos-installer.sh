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

# Start from a clean output directory, then build the native .app bundle.
rm -rf dist
bash ./scripts/build-macos-app.sh

APP_PATH="$ROOT_DIR/dist/Trixy.app"
if [[ ! -d "$APP_PATH" ]]; then
  echo "Expected app bundle was not created: $APP_PATH" >&2
  exit 1
fi

# Ad-hoc sign the app so the bundle is internally consistent. This is not
# Developer ID signing/notarization, which requires Apple credentials.
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP_PATH"
  codesign --verify --deep --strict "$APP_PATH"
fi

VERSION="$(awk -F ' *= *' '/^version *=/ {gsub(/\"/, "", $2); print $2; exit}' Cargo.toml)"
if [[ -z "$VERSION" ]]; then
  VERSION="unknown"
fi

ARCH="$(uname -m)"
DMG_PATH="$ROOT_DIR/dist/Trixy-${VERSION}-${ARCH}.dmg"
STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/trixy-dmg.XXXXXX")"
trap 'rm -rf "$STAGING_DIR"' EXIT

# A conventional drag-to-Applications DMG: the app plus an Applications link.
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

echo
echo "macOS installer build complete:"
echo "  $DMG_PATH"
