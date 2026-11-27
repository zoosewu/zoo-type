#!/bin/sh
# Update only artwork in an already registered input method; preserve its binary,
# metadata, enabled-source list, current selection and user data.
set -eu
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
NAME="動物園輸入法"
OUTPUT=target/macos
case "${1:-}" in
    --dev) NAME="動物園輸入法 開發版"; OUTPUT=target/macos-dev ;;
    "") ;;
    *) echo "用法：sh scripts/update-macos-icons.sh [--dev]" >&2; exit 2 ;;
esac
SOURCE="$ROOT/$OUTPUT/$NAME.app/Contents/Resources"
DEST="$HOME/Library/Input Methods/$NAME.app"
[ -d "$DEST" ] || { echo "尚未安裝 ${NAME}。" >&2; exit 1; }
for icon in menu.tiff menu-dark.tiff zoo-type.icns app-icon.png; do
    [ -f "$SOURCE/$icon" ] || { echo "缺少圖示：${icon}，請先建置。" >&2; exit 1; }
done
codesign --verify --strict "$DEST"
STAGE=$(mktemp -d "$HOME/Library/Input Methods/.zoo-type-icons.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
COPY="$STAGE/$NAME.app"
ditto "$DEST" "$COPY"
for icon in menu.tiff menu-dark.tiff zoo-type.icns app-icon.png; do
    cp "$SOURCE/$icon" "$COPY/Contents/Resources/$icon"
done
codesign --force --sign - "$COPY"
codesign --verify --strict "$COPY"
BACKUP="$DEST.previous-icons-$(date +%Y%m%d%H%M%S)-$$"
mv "$DEST" "$BACKUP"
if ! mv "$COPY" "$DEST"; then mv "$BACKUP" "$DEST"; exit 1; fi
echo "圖示已更新，已保留原執行檔與輸入法登記。"
echo "舊版備份：$BACKUP"
echo "macOS 若仍顯示舊圖示，可先切換輸入法；下次登入也會重新載入。"
