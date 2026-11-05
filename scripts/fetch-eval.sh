#!/usr/bin/env sh
# 下載評測用的「詞 → 注音」對照（libchewing-data，LGPL-2.1-or-later）到 assets/eval
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/assets/eval"
mkdir -p "$DEST"
URL="https://codeberg.org/chewing/libchewing-data/raw/branch/main/dict/chewing/tsi.csv"
echo "下載 $URL"
curl -fsSL -A "zct-dev" -o "$DEST/tsi.csv" "$URL"
echo "完成：$(grep -c . "$DEST/tsi.csv") 行"
