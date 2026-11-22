#!/bin/sh
# Run from the repository root after cargo build --release --locked.
set -eu
version=$(sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -1)
app=target/release/Rectangle.app
mkdir -p "$app/Contents/MacOS"
cp target/release/rectangle "$app/Contents/MacOS/rectangle"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>rectangle</string>
<key>CFBundleIdentifier</key><string>com.mdsmithson.rectangle</string>
<key>CFBundleName</key><string>Rectangle</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSAppleEventsUsageDescription</key><string>Rectangle opens Terminal for server monitoring and native file save dialogs.</string>
</dict></plist>
PLIST
# This is a local ad-hoc signature, not Developer ID signing or notarization.
codesign --force --sign - "$app/Contents/MacOS/rectangle"
printf '%s\n' "$app"
