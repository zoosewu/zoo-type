#!/bin/sh
# 安裝到目前使用者的 Input Methods；保留設定與學習資料。
set -eu
[ "$(uname -s)" = Darwin ] || { echo "需要 macOS" >&2; exit 1; }
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
NAME="動物園輸入法"
OUTPUT=target/macos
CHANNEL=--official
case "${1:-}" in
    --dev) NAME="動物園輸入法 開發版"; OUTPUT=target/macos-dev; CHANNEL=--dev ;;
    "") ;;
    *) echo "用法：sh scripts/install-macos.sh [--dev]" >&2; exit 2 ;;
esac
SOURCE="$ROOT/$OUTPUT/$NAME.app"
# Release 壓縮包中腳本位於頂層，.app 就在旁邊。
if [ ! -d "$SOURCE" ]; then
    SOURCE="$(dirname "$0")/$NAME.app"
fi
[ -d "$SOURCE" ] || { echo "找不到 $NAME.app，請先建置。" >&2; exit 1; }
# 輸入法裝在目前帳號、登記在目前帳號的輸入來源清單：用 sudo 會變成 root 的，系統找不到
if [ "$(id -u)" -eq 0 ]; then
    echo "請不要用 sudo，直接執行：sh \"$0\" ${1:-}" >&2
    exit 1
fi
# 先前用 sudo 安裝留下 root 擁有的檔案：輸入法讀寫不了，請先改回自己的
for DIR in "$HOME/Library/Input Methods" "$HOME/Library/Application Support/ZooType" "$HOME/Library/Application Support/ZooType-dev"; do
    if [ -e "$DIR" ] && [ -n "$(find "$DIR" ! -user "$(id -u)" -print -quit 2>/dev/null)" ]; then
        echo "「${DIR}」裡有不屬於你的檔案（先前用 sudo 安裝留下的），請先執行：" >&2
        echo "  sudo chown -R \"$(id -un)\" \"$DIR\"" >&2
        echo "再重新執行 sh \"$0\" ${1:-}" >&2
        exit 1
    fi
done
codesign --verify --strict "$SOURCE"
DEST="$HOME/Library/Input Methods/$NAME.app"
mkdir -p "$HOME/Library/Input Methods"
STAGE=$(mktemp -d "$HOME/Library/Input Methods/.zoo-type-install.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
ditto "$SOURCE" "$STAGE/$NAME.app"
codesign --verify --strict "$STAGE/$NAME.app"
BACKUP=""
if [ -d "$DEST" ]; then
    echo "升級前請先切換至其他輸入法，並登出再登入以卸載舊版。" >&2
    echo "將以新版取代 ${DEST}；資料設定會保留。" >&2
    # 不直接覆寫可能正在執行的 Mach-O。
    STAMP="$(date +%Y%m%d%H%M%S)-$$"
    BACKUP="$DEST.previous-$STAMP"
    mv "$DEST" "$BACKUP"
fi
if ! mv "$STAGE/$NAME.app" "$DEST"; then
    if [ -n "$BACKUP" ]; then mv "$BACKUP" "$DEST"; fi
    exit 1
fi
if ! "$DEST/Contents/MacOS/zoo-type" --register; then
    echo "檔案已安裝，但系統註冊未完成。詳細錯誤見上方輸出。" >&2
    printf '若偏好設定服務拒絕寫入，可在桌面 Terminal 執行：sh "%s/repair-macos-registration.command" %s\n' "$(dirname "$0")" "$CHANNEL" >&2
    exit 1
fi
echo "安裝完成，已驗證輸入來源啟用清單。請從上方輸入法選單選擇「${NAME}」。"
echo "若清單尚未出現，請登出再登入。"
