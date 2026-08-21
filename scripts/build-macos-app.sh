#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

cargo build --release --bin trixy

APP="dist/Trixy.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/trixy "$APP/Contents/MacOS/trixy"
cp assets/trixy-icon.icns "$APP/Contents/Resources/trixy-icon.icns"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>trixy</string>
  <key>CFBundleIdentifier</key><string>org.trixy.desktop</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Trixy</string>
  <key>CFBundleDisplayName</key><string>Trixy</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>trixy-icon.icns</string>
  <key>CFBundleShortVersionString</key><string>0.6.1</string>
  <key>CFBundleVersion</key><string>6</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Keep the standalone app usable for local testing. The installer script signs
# and verifies it again immediately before creating the DMG.
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP"
fi

echo "Built $APP"
echo "You can copy that .app to another Mac with the same CPU architecture."
