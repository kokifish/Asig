#!/usr/bin/env bash
# 编译 release 并打包成 Asig.app(菜单栏 accessory:LSUIElement=true,不占 Dock)。
# 用法: ./scripts/make-app.sh   产物: build/Asig.app
set -euo pipefail

cd "$(dirname "$0")/.."

APP_NAME="Asig"
BIN_NAME="agent-light"            # Cargo [[bin]] 名
BUNDLE_ID="com.kokifish.asig"
VERSION="0.1.0"

echo "==> cargo build --release -p agent-light"
cargo build --release -p agent-light

APP="build/${APP_NAME}.app"
CONTENTS="${APP}/Contents"
MACOS="${CONTENTS}/MacOS"
RESOURCES="${CONTENTS}/Resources"

echo "==> 组装 ${APP}"
rm -rf "${APP}"
mkdir -p "${MACOS}" "${RESOURCES}"

cp "target/release/${BIN_NAME}" "${MACOS}/${BIN_NAME}"

cat > "${CONTENTS}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>            <string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key>     <string>${APP_NAME}</string>
  <key>CFBundleIdentifier</key>      <string>${BUNDLE_ID}</string>
  <key>CFBundleVersion</key>         <string>${VERSION}</string>
  <key>CFBundleShortVersionString</key> <string>${VERSION}</string>
  <key>CFBundlePackageType</key>     <string>APPL</string>
  <key>CFBundleExecutable</key>      <string>${BIN_NAME}</string>
  <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
  <key>LSMinimumSystemVersion</key>  <string>11.0</string>
  <key>LSUIElement</key>             <true/>
  <key>NSHighResolutionCapable</key> <true/>
  <key>NSRequiresAquaSystemAppearance</key> <false/>
</dict>
</plist>
PLIST

echo "==> 完成:${APP}"
echo "    运行测试:  open ${APP}"
echo "    安装:      cp -R ${APP} /Applications/"
