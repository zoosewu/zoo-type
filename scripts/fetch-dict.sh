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

# 英文單字表（中英混打用）：dwyl/english-words，Unlicense（公眾領域）
WORDS="https://raw.githubusercontent.com/dwyl/english-words/master/words_alpha.txt"
echo "下載 $WORDS"
curl -fsSL -A "zct-dev" "$WORDS" | tr -d '\r' | awk 'length($0) >= 2 && length($0) <= 20' | LC_ALL=C sort -u > "$DEST/english.txt"

# Emoji 繁中關鍵字：Unicode CLDR annotations，Unicode-3.0 授權
EMOJI="https://raw.githubusercontent.com/unicode-org/cldr-json/main/cldr-json/cldr-annotations-full/annotations/zh-Hant/annotations.json"
echo "下載 $EMOJI"
curl -fsSL -A "zct-dev" -o "$DEST/emoji-zh-Hant.json" "$EMOJI"

# 詞 → 讀音（設定程式自動填注音、選字評測）：libchewing-data，LGPL-2.1-or-later
TSI="https://codeberg.org/chewing/libchewing-data/raw/branch/main/dict/chewing/tsi.csv"
echo "下載 $TSI"
curl -fsSL -A "zct-dev" -o "$DEST/tsi.csv" "$TSI"

# 智慧選字（T0）的字 n-gram 模型：本專案用 zct-train 從中文維基百科與生成的口語句子訓練，CC-BY-SA-4.0
# 放在 pre-release「model-<日期>」（不會被自動更新當成新版）；換模型時同時改日期與 SHA256，見 docs/architecture.md
MODEL="model-20261116"
MODEL_SHA256="8f68b5ef5fcaa137f72c1f837780bcedf65c079854375360582b892ccd900bd5"
NGRAM="https://github.com/zoosewu/zoo-type/releases/download/$MODEL/ngram.bin"
echo "下載 $NGRAM"
curl -fsSL -A "zct-dev" -o "$TMP/ngram.bin" "$NGRAM"
if command -v sha256sum >/dev/null 2>&1; then
    GOT=$(sha256sum "$TMP/ngram.bin" | cut -d' ' -f1)
else
    GOT=$(shasum -a 256 "$TMP/ngram.bin" | cut -d' ' -f1)
fi
if [ "$GOT" != "$MODEL_SHA256" ]; then
    echo "ngram.bin 的 SHA256 不符：$GOT" >&2
    exit 1
fi
mv "$TMP/ngram.bin" "$DEST/ngram.bin"
echo "完成：$DEST"
ls -la "$DEST"
