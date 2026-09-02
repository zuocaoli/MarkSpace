#!/bin/sh
# 组装 MarkSpace.app：用法 ./bundle.sh <release 二进制路径> <版本号> [输出目录]
# 在仓库根目录执行；产物为 <输出目录>/MarkSpace.app
set -e

BIN="$1"
VERSION="$2"
OUT="${3:-.}"

if [ -z "$BIN" ] || [ -z "$VERSION" ]; then
    echo "用法: $0 <二进制路径> <版本号> [输出目录]" >&2
    exit 1
fi

# 脚本位于 packaging/macos/，仓库根为上两级
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APP="$OUT/MarkSpace.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/MarkSpace"
chmod +x "$APP/Contents/MacOS/MarkSpace"
cp "$ROOT/assets/app.icns" "$APP/Contents/Resources/AppIcon.icns"
sed "s/__VERSION__/$VERSION/g" "$ROOT/packaging/macos/Info.plist" \
    > "$APP/Contents/Info.plist"

echo "$APP"
