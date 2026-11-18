# macOS 輸入法

macOS 前端使用 Swift / InputMethodKit，透過 cbindgen 產生的 C ABI 直接呼叫 Rust 核心。
建置會產生可安裝到 `~/Library/Input Methods` 的「動物園輸入法.app」，也保留模擬器與評測工具。

## 建置與安裝

需要 macOS 15 以上、Xcode Command Line Tools（`xcode-select --install`）、Python 3，以及符合 `Cargo.toml` 的 Rust / Cargo。
GitHub Release 使用 rustup 官方 stable 工具鏈。自行使用 Homebrew Rust 時，其預編譯標準庫可能要求較新的 macOS；
若 linker 顯示最低版本警告，不應將該本機產物當作可支援舊版 macOS 的發佈檔。

```sh
sh scripts/fetch-dict.sh
sh scripts/build-macos.sh --check
sh scripts/install-macos.sh
```

在「系統設定 → 鍵盤 → 文字輸入 → 編輯 → ＋ → 繁體中文」加入「動物園輸入法」。
若清單未出現，登出再登入。安裝不需要 sudo；設定與學習資料保留在 `~/Library/Application Support/ZooType`，
也支援既有的 `location.toml` 與 `ZCT_HOME`。升級後應登出再登入，讓載入中的舊版程序結束。
安裝腳本會將原有 .app 改名保留為 `.app.previous-<時間>`，不直接覆寫正在載入的執行檔。

```sh
# 與正式版分開的 Bundle ID、連線名稱、資料目錄和產物
sh scripts/build-macos.sh --check --dev
sh scripts/install-macos.sh --dev
```

正式版產物：`target/macos/動物園輸入法.app`；開發版：`target/macos-dev/動物園輸入法 開發版.app`。
不要用 Windows 的 PowerShell 安裝腳本或 `cargo build-dev` 安裝 macOS 版。

## 已實作的輸入行為

- 大千注音、inline 組字、游標與選字範圍、Enter 送出、Esc 取消。
- 候選窗沿用 Windows 共用的 theme：配色、字級、數字欄與整列選取框，支援直式、橫式、多欄與固頂標記。
- 一般 App 使用 inline 組字；終端機或啟用組字提示列時才另外顯示，避免重複提示。
- 單按左右 Shift 切中英、全半形、繁簡、標點、數字鍵盤、Emoji 與核心詞庫學習。
- Command / Option / Control 組合鍵放行；使用者明確綁定的快捷鍵由核心處理。
- 切換 App / 輸入法時結束組字；輸出會套用繁簡設定。未完成的注音音節會取消。
- 每個輸入 context 獨立 session，共用引擎與遊戲模式狀態，定期及失焦時儲存學習資料。
- 依 Bundle ID 套用 `apps.toml`，每次切入時讀取設定；上方輸入法選單的「設定…」可開啟原生設定視窗。
- 重新選字使用客戶端的選取範圍；不支援讀取選取文字的 App 會放棄並提示音。

原生設定視窗包含一般、快捷鍵、外觀、App 規則、詞庫、學習、同步、AI 與關於九頁。透過 `zct-preferences` 與 Windows 共用 44 個全域設定、10 個 App 規則欄位、預設值、驗證、儲存及管理操作介面。Windows 的「全部設定」使用相同欄位目錄。

macOS 尚未提供 AI 執行器、遊戲偵測、自動更新及組字提示列快捷鍵，因此相關欄位顯示為灰色並保留既有值。詞庫新增、編輯、讀音查詢、優先／封鎖、統計、備份與還原可直接從設定頁操作。資料位置可在同步頁變更；使用 `ZCT_HOME` 時位置會鎖定。

儲存前會驗證整份設定並比對載入版本；其他程式或視窗修改後，須先按「重新載入」，避免覆寫。備份還原或資料搬移後，請切換輸入法再切回以建立新的輸入 session。
目前尚未移植 AI 背景請求與 Windows 啟發式遊戲偵測，候選窗以鍵盤操作。git 背景同步與詞庫管理 GUI 已提供。

## 設定入口與圖示更新

選取動物園輸入法後，點上方輸入法選單的「動」圖示 →「設定…」。
也可雙擊已在執行的 .app，或直接執行：

```sh
"target/macos/動物園輸入法.app/Contents/MacOS/zoo-type" --settings
# 開發版
"target/macos-dev/動物園輸入法 開發版.app/Contents/MacOS/zoo-type" --settings
```

Release 解壓後也提供「開啟設定.command」。同一個輸入法程序已執行時會將設定視窗叫到前面，不會另開引擎寫入學習資料。
輸入法使用獨立的 16 點 TIFF（含 Retina）；淺色與深色選單皆為正方形白底、黑色「動」字，接近 Windows 的樣式。Finder 使用 ICNS，不把大尺寸 App 圖示用於選單。
只更新既有開發版的圖示可用 `sh scripts/update-macos-icons.sh --dev`，保留原執行檔與輸入法登記，不需要再次修改啟用清單。
註冊時會先啟用父輸入法，再啟用注音模式，最後驗證 `com.apple.HIToolbox` 的 `AppleEnabledInputSources`。
部分 macOS 環境會讓 TIS 啟用 API 回報成功卻沒有更新清單；這時只補入動物園缺少的項目，保留其他輸入來源與順序，
並在 `~/Library/Application Support/ZooType[-dev]/local/registration-backups` 儲存修改前的清單。
無法驗證清單時會回報註冊失敗，而不只根據 API 退出碼宣告成功。
如果工具或自動化程序無法存取 `cfprefsd`，不要再反覆登出或重裝；在登入桌面的 Terminal 執行
`sh scripts/repair-macos-registration.command --dev`（正式版用 `--official`）。Release 包裡也有同名腳本。
它會先匯出整份 HIToolbox 備份，只追加動物園缺少的項目，驗證其他來源仍在，最後重新載入 `TextInputMenuAgent`；
不切換目前輸入法、不結束正在輸入文字的 App。
相同狀況的上游實測紀錄：[Meltype #21](https://github.com/yksr-melt/Meltype/issues/21)。

新版以單一注音模式提供明確的名稱與圖示。從舊版更新後請登出再登入；若仍保留舊輸入來源項目，
在系統設定的輸入方式清單移除舊項目，再加入新版「動物園輸入法」（或開發版）。

## GitHub Release

推送與 `Cargo.toml` workspace 版本一致的 `v*` 標籤會觸發 Release workflow。
Windows、macOS Apple Silicon 和 Intel 分別測試與建置；全部成功後統一發布同一個 Release。

- Apple Silicon 輸入法：`zoo-type-<版本>-macos-arm64.zip`
- Intel 輸入法：`zoo-type-<版本>-macos-x64.zip`
- 另附 `-tools.tar.gz` 命令列工具包；每個壓縮包皆附 `.sha256`。

輸入法壓縮包內包含 .app、`install.sh` 與 README，執行 `sh install.sh` 安裝到目前使用者的 Input Methods。
建置會使用 ad-hoc 簽章以滿足執行需求，尚未設定 Developer ID 簽章與 Apple 公證。
下載後若被 Gatekeeper 阻擋，確認來源後至「系統設定 → 隱私權與安全性」允許；腳本不會移除隔離標記或關閉系統安全機制。
Windows 原有 `ZOO_TYPE_SIGNING_KEY` 仍為必要，目前沒有額外的 Apple Secrets 需求。

本機打包：`sh scripts/package-macos.sh`，產物在 `target/installer`。
工具包內的 `bin/zoo-type-sim`、`bin/zct-eval` 可直接讀取隨附資源，亦可用 `ZCT_RESOURCE_ROOT` 覆寫資源目錄。

## 驗證範圍

建置腳本的 `--check` 執行 Rust 格式、Clippy 與測試；每次建置 .app 都會驗證簽章並執行 Swift / Rust 自我測試。
自我測試在隔離的暫存 `ZCT_HOME` 驗證組字、候選、送字、Command 放行、重新選字與失焦，不註冊輸入法。
打包也會再次執行此測試。CI 會在 arm64 與 x64 runner 分別執行。

`--ui-self-test <輸出資料夾>` 會離屏繪製設定頁與直式／橫式／多欄候選窗，並驗證選單圖示的邏輯尺寸是 16×16。

這些測試不代表已驗證所有真實 App 的 IMK 行為。安裝後應在 TextEdit、Safari、VS Code、Terminal 手動檢查：

1. 輸入 `wu0 fu4` 顯示「天氣」，Enter 只送出一次；Esc 清空組字。
2. ↓ 開啟候選，方向鍵移動、數字選字，候選窗跟隨游標且不取得焦點。
3. 單按 Shift 切換中英；Shift 搭配字母與 Command 快捷鍵不誤切換。
4. 組字時切換 App 或輸入法，不重複送字、不殘留候選窗。
5. 外接螢幕、全螢幕、視窗邊緣、密碼欄位與重新選字的 App 相容性。

卸載時先切換到其他輸入法，在系統設定移除項目，再刪除 .app 並登出登入；使用者資料不會自動刪除。


## Git 同步

macOS 與 Windows 使用 `zct-preferences::git` 的同一個同步流程。從輸入法選單開啟「設定… → 同步」，填入專用 repository 的 SSH 或 HTTPS URL，儲存後按「立即同步」。開啟自動同步後，輸入法會按設定的分鐘間隔在背景執行；排程每 30 秒檢查一次。留空遠端只建立本機 commit。

需先安裝系統 git，並在終端機完成 SSH key、主機信任或 HTTPS credential helper 的設定。輸入法不提供互動式密碼輸入。失敗原因會顯示在同步頁，並寫入資料目錄的 `local/sync-status.txt`。同步不會阻塞輸入法主執行緒；進行期間不能搬移資料位置、還原備份或從設定視窗覆寫設定。

同步使用 `main` 分支。第一次加入已有內容的遠端時會採用遠端設定，再合併本機詞庫。同步內容包括設定、App 規則、可匯出的使用者詞庫、英文單字與統計；`local/` 中的單機資料不會提交，chewing 的二進位自動學習紀錄不做合併。設定檔衝突會停止並撤銷此次合併，不會強制推送；請在資料目錄使用 git 解決分歧後再同步。

同步完成後按「重新載入」查看遠端設定，切換輸入法後套用新的輸入設定。
