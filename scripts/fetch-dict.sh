#!/usr/bin/env sh
# 下載 libchewing 系統詞庫到 assets/dict（開發與測試用）
# 授權：詞庫 LGPL-2.1-or-later，語言模型 CC-BY-4.0（見 libchewing-data）
set -eu
VERSION="${1:-2026.10.27}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/assets/dict"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

URL="https://codeberg.org/chewing/libchewing-data/releases/download/v$VERSION/libchewing-data-$VERSION-Generic.zip"
echo "下載 $URL"
curl -fsSL -A "zct-dev" -o "$TMP/data.zip" "$URL"
unzip -q "$TMP/data.zip" -d "$TMP"
mkdir -p "$DEST"
cp "$TMP/libchewing-data-$VERSION-Generic/share/libchewing/"* "$DEST/"
echo "$VERSION" > "$DEST/VERSION"
echo "完成：$DEST"
ls -la "$DEST"
