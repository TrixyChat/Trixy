#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

cargo build --release --bin trixy

APP="$ROOT_DIR/dist/Trixy.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT_DIR/target/release/trixy" "$APP/Contents/MacOS/trixy"
cp "$ROOT_DIR/assets/trixy-icon.icns" "$APP/Contents/Resources/trixy-icon.icns"
chmod 755 "$APP/Contents/MacOS/trixy"

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n 1)"
if [ -z "$VERSION" ]; then
  VERSION="0.0.0"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
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
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Validate the bundle metadata before packaging. plutil is part of macOS.
if command -v plutil >/dev/null 2>&1; then
  plutil -lint "$APP/Contents/Info.plist"
fi

# Ad-hoc signing requires no Apple certificate. It keeps the generated bundle
# internally consistent; Developer ID signing/notarization can be added later.
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP"
  codesign --verify --deep --strict "$APP"
fi

echo "Built $APP"
