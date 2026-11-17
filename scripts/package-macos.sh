#!/bin/sh
# 先執行 build-macos.sh；打包 InputMethodKit .app 與可攜式工具。
set -eu
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT"
[ "$(uname -s)" = Darwin ] || { echo "需要 macOS" >&2; exit 1; }
VERSION=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\([^"]*\)"/\1/p' Cargo.toml)
case "$(uname -m)" in
    arm64) ARCH=arm64 ;;
    x86_64) ARCH=x64 ;;
    *) echo "不支援的架構" >&2; exit 1 ;;
esac
[ -n "$VERSION" ] || { echo "找不到 workspace 版本" >&2; exit 1; }
NAME="zoo-type-$VERSION-macos-$ARCH-tools"
DEST="$ROOT/target/installer"
mkdir -p "$DEST"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
PACKAGE="$STAGE/$NAME"
mkdir -p "$PACKAGE/bin" "$PACKAGE/assets" "$PACKAGE/tests/eval"
cp target/macos/release/examples/sim "$PACKAGE/bin/zoo-type-sim"
cp target/macos/release/zct-eval "$PACKAGE/bin/zct-eval"
cp -R assets/dict "$PACKAGE/assets/dict"
cp tests/eval/corpus.txt "$PACKAGE/tests/eval/"
cp LICENSE-MIT LICENSE-APACHE Cargo.lock "$PACKAGE/"
cp macos/THIRD-PARTY-NOTICES.txt "$PACKAGE/"
cat > "$PACKAGE/README.txt" <<EOF
動物園輸入法 $VERSION — macOS $ARCH 命令列工具

這不是可安裝的輸入法；InputMethodKit 前端尚未完成。
GitHub Release 的工具最低需求為 macOS 15。
解壓後保留資料夾結構，從終端機執行：
  bin/zoo-type-sim "wu0 fu4" "<enter>"
  bin/zct-eval
詞庫與評測語料會從壓縮包內讀取，不需要原始碼 checkout。
可使用 ZCT_RESOURCE_ROOT 指定另一個含 assets/dict 與 tests/eval 的資料夾。
工具未做 Developer ID 簽章或公證。
原始碼與重新建置方式：https://github.com/zoosewu/zoo-type/tree/v$VERSION
EOF
# 在打包前從不同工作目錄執行，確認資料不依賴 CI checkout 的絕對路徑。
(cd "$STAGE" && env -u ZCT_RESOURCE_ROOT ZCT_SIM_LEARNING="$STAGE/learning" "$PACKAGE/bin/zoo-type-sim" "wu0 fu4" "<enter>" > "$STAGE/sim.log")
grep -q '送出="天氣"' "$STAGE/sim.log"
tar -czf "$DEST/$NAME.tar.gz" -C "$STAGE" "$NAME"
(cd "$DEST" && shasum -a 256 "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
echo "$DEST/$NAME.tar.gz"

APP="$ROOT/target/macos/動物園輸入法.app"
codesign --verify --strict "$APP"
"$APP/Contents/MacOS/zoo-type" --self-test
APP_NAME="zoo-type-$VERSION-macos-$ARCH"
APP_PACKAGE="$STAGE/$APP_NAME"
mkdir -p "$APP_PACKAGE"
ditto "$APP" "$APP_PACKAGE/動物園輸入法.app"
cp scripts/install-macos.sh "$APP_PACKAGE/install.sh"
cp scripts/repair-macos-registration.command "$APP_PACKAGE/repair-macos-registration.command"
cat > "$APP_PACKAGE/開啟設定.command" <<'SETTINGS'
#!/bin/sh
set -eu
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
exec "$ROOT/動物園輸入法.app/Contents/MacOS/zoo-type" --settings
SETTINGS
chmod +x "$APP_PACKAGE/開啟設定.command"
cp LICENSE-MIT LICENSE-APACHE "$APP_PACKAGE/"
cat > "$APP_PACKAGE/README.txt" <<EOF
動物園輸入法 $VERSION — macOS ${ARCH}（macOS 15 以上）

這個壓縮包包含 InputMethodKit 輸入法 .app 與內建詞庫。

安裝：
1. 解壓後，在終端機進入這個資料夾，執行 sh install.sh（不需要 sudo）。
   也可自行把「動物園輸入法.app」放到 ~/Library/Input Methods/。
2. 在系統設定 → 鍵盤 → 文字輸入 → 編輯 → ＋ → 繁體中文，加入「動物園輸入法」。
3. 若清單尚未出現，請登出再登入。

這是初版 macOS 前端，尚未做 Developer ID 簽章或 Apple 公證。
若 macOS 阻擋開啟，請先確認下載來源，再至系統設定 → 隱私權與安全性允許。
升級前請切換到其他輸入法；安裝後登出再登入，讓舊版程序結束。

中文／英文：單按 Shift。選字：↓ 開啟，方向鍵移動，Enter 或數字確認。
候選窗目前以鍵盤操作。Control / Option / Command 組合鍵預設交回 App。
設定檔與 App 規則在 ~/Library/Application Support/ZooType/。
點上方「動」圖示 →「設定…」開啟原生設定視窗，也可雙擊本包的「開啟設定.command」。
一般、快捷鍵、外觀預覽與 App 規則可直接修改，其他欄位可從關於頁開啟進階設定檔。
從舊版升級後請登出再登入；若仍出現舊輸入來源，先移除後再加入新版。
學習資料定期存檔，並在切換輸入法時存檔。
AI、git 同步、詞庫管理 GUI 與 Windows 的啟發式遊戲偵測尚未移植。
卸載：切換到其他輸入法，移除輸入方式清單中的項目，刪除該 .app，再登出登入。
使用者資料不會隨著移除 .app 而刪除。

原始碼與重新建置方式：https://github.com/zoosewu/zoo-type/tree/v$VERSION
EOF
ditto -c -k --sequesterRsrc --keepParent "$APP_PACKAGE" "$DEST/$APP_NAME.zip"
(cd "$DEST" && shasum -a 256 "$APP_NAME.zip" > "$APP_NAME.zip.sha256")
echo "$DEST/$APP_NAME.zip"
