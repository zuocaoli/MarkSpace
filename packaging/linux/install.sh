#!/bin/sh
# 用户级安装 MarkSpace（无需 root）：二进制、图标、.desktop 启动器。
# 用法：解压发布包后在解压目录内执行 ./install.sh
set -e

DIR=$(cd "$(dirname "$0")" && pwd)
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
install -m 755 "$DIR/MarkSpace" "$BIN_DIR/MarkSpace"
install -m 644 "$DIR/MarkSpace.desktop" "$APP_DIR/MarkSpace.desktop"
install -m 644 "$DIR/MarkSpace.png" "$ICON_DIR/MarkSpace.png"

# 刷新桌面数据库与图标缓存（工具不存在时静默跳过）
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APP_DIR" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && \
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "已安装：$BIN_DIR/MarkSpace"
echo "启动器与图标已就绪（应用菜单中搜索 MarkSpace）。"
echo "若终端无法直接运行 MarkSpace，请确认 $BIN_DIR 在 PATH 中。"
