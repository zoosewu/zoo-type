#!/usr/bin/env sh
# 下載智慧選字（T0）的字 n-gram 模型到 assets/dict/ngram.bin
#
# 模型附在每個正式版 Release 上（ngram.bin 與 ngram.bin.sha256），這裡取最新正式版的那一份；
# Release workflow 再把它附到新版上，模型就一版一版傳下去。換模型：把新的兩個檔案換到目前最新的 Release 上。
# 模型由 zct-train 從中文維基百科與生成的口語句子訓練，CC-BY-SA-4.0。
#
# ZCT_MODEL_URL 可指定另一個位置（同一個資料夾要有 ngram.bin.sha256）
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/assets/dict"
BASE="${ZCT_MODEL_URL:-https://github.com/zoosewu/zoo-type/releases/latest/download}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "下載 $BASE/ngram.bin"
curl -fsSL -A "zct-dev" -o "$TMP/ngram.bin" "$BASE/ngram.bin"
curl -fsSL -A "zct-dev" -o "$TMP/ngram.bin.sha256" "$BASE/ngram.bin.sha256"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$TMP" && sha256sum -c ngram.bin.sha256)
else
    (cd "$TMP" && shasum -a 256 -c ngram.bin.sha256)
fi
mkdir -p "$DEST"
mv "$TMP/ngram.bin" "$DEST/ngram.bin"
