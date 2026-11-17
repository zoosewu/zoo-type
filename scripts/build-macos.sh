#!/bin/sh
# macOS 輸入法、共用引擎與工具建置。
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT"

usage() {
    echo "用法：sh scripts/build-macos.sh [--check] [--dev]"
    echo "  預設：建置 release InputMethodKit .app、引擎與工具"
    echo "  --check：格式、Clippy、測試，再建置 release"
    echo "  --dev：使用獨立的開發版資料與 target/macos-dev"
}

CHECK=false
DEV=false
for arg in "$@"; do
    case "$arg" in
        --check) CHECK=true ;;
        --dev) DEV=true ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done

if [ "$(uname -s)" != Darwin ]; then
    echo "這個腳本需要在 macOS 上執行。" >&2
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "找不到 cargo。請先安裝 Rust（版本需求見 Cargo.toml 的 rust-version）。" >&2
    exit 1
fi
if ! xcrun --find clang >/dev/null 2>&1; then
    echo "請先執行 xcode-select --install 安裝 Command Line Tools。" >&2
    exit 1
fi

set -- -p zct-core -p zct-ai -p zct-eval -p zct-ffi -p zct-preferences --locked
OUTPUT=target/macos
APP_NAME="動物園輸入法"
BUNDLE_ID=com.zoosewu.inputmethod.ZooType
if [ "$DEV" = true ]; then
    OUTPUT=target/macos-dev
    APP_NAME="動物園輸入法 開發版"
    BUNDLE_ID=com.zoosewu.inputmethod.ZooType.dev
    set -- "$@" --features zct-ffi/dev
fi
set -- "$@" --target-dir "$OUTPUT"
export ZCT_FFI_HEADER="$ROOT/$OUTPUT/include/zct.h"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}"

if [ "$CHECK" = true ]; then
    if [ ! -f assets/dict/VERSION ]; then
        echo "缺少測試詞庫，請先執行 sh scripts/fetch-dict.sh。" >&2
        exit 1
    fi
    cargo fmt --all --check
    cargo clippy "$@" --all-targets -- -D warnings
    cargo test "$@"
fi
cargo build "$@" --release --lib --bins --examples
APP="$ROOT/$OUTPUT/$APP_NAME.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
VERSION=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\([^"]*\)"/\1/p' Cargo.toml)
xcrun swiftc -swift-version 5 -O -module-name ZooType \
    -target "$(uname -m)-apple-macosx$MACOSX_DEPLOYMENT_TARGET" \
    -module-cache-path "$ROOT/$OUTPUT/swift-module-cache" \
    -import-objc-header "$ZCT_FFI_HEADER" \
    macos/Sources/*.swift -L "$ROOT/$OUTPUT/release" -lzct_ffi \
    -framework AppKit -framework InputMethodKit -framework Carbon -liconv \
    -o "$APP/Contents/MacOS/zoo-type"
python3 scripts/macos-plist.py "$APP" "$APP_NAME" "$BUNDLE_ID" "$VERSION" "$MACOSX_DEPLOYMENT_TARGET"
ditto assets/dict "$APP/Contents/Resources/dict"
cp LICENSE-MIT LICENSE-APACHE Cargo.lock macos/THIRD-PARTY-NOTICES.txt "$APP/Contents/Resources/"
xcrun swift -module-cache-path "$ROOT/$OUTPUT/swift-module-cache" scripts/macos-icons.swift "$APP/Contents/Resources"
ICONSET="$ROOT/$OUTPUT/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$APP/Contents/Resources/app-icon.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$APP/Contents/Resources/app-icon.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
python3 scripts/macos-icns.py "$ICONSET" "$APP/Contents/Resources/zoo-type.icns"
# 本機與 CI 使用 ad-hoc 簽章；Developer ID 與公證另行配置。
codesign --force --sign - "$APP"
codesign --verify --strict "$APP"
"$APP/Contents/MacOS/zoo-type" --self-test
echo "macOS 輸入法建置完成：$APP"
