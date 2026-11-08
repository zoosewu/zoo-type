# ZCT 開發計畫

已完成 M0～M8.3（Windows 版日常可用），細節與檢討見 [archive.md](archive.md)。

## 零散待辦
- [ ] x86 / ARM64 DLL 建置（原 M2 Windows 最小可用）
- [ ] 實測：設定程式本身能用 ZCT 打中文（Slint 輸入法支援）（原 M5 設定程式與候選窗主題）
- [ ] 服務程序還沒啟動時，最前面幾個按鍵會漏給 App（DLL 啟動服務的期間）（原 M8.2：打字回饋修正）
- [ ] 設定程式詞庫頁加上英文單字清單（目前可用 Shift+Delete 或直接編輯 dict/english.txt）（原 M8.2：打字回饋修正）
- [ ] 手動遊戲模式時 DLL 直接放行、不經 IPC（現在每鍵仍問服務程序，正常 <1ms，服務卡住最多 100ms）（原 M8.2：打字回饋修正）
- [ ] tsf_probe 會把快捷鍵切換的結果寫進使用者真正的 `apps.toml` → 改用獨立的資料資料夾（`ZCT_HOME`）

## M10 Release 與 GitHub Actions（repo：zoo-type；分支 `m10`；初版完成前不發 release）

前置：已完成（M8.3 合併回 main）。按鍵診斷保留（`local/debug-keys` 存在才記錄非字元鍵）

### 0. 新舊版本並存（取代協定向下相容；使用者決定 2026-11-08）
- [x] 每個版本裝在 `Program Files\ZCT\<版本資料夾>\`，根目錄 `current.txt` 記錄目前版本與協定版本
- [x] 連線名稱與單一實例 mutex 帶協定版本：不同協定的服務程序可以同時存在
- [x] 舊版服務程序：協定相同 → 讓位給新版；協定不同 → 唯讀（不存學習、不寫設定、不同步），沒有連線時結束
- [x] `zct-service --protocol` 印出協定版本（安裝程式寫 current.txt 用）
- [x] install-dev.ps1：裝到 `dev-<日期時間>-<commit>`、不強制結束服務程序；舊版整個資料夾刪除或排到重開機刪除
- [ ] 實機驗證：連續安裝兩次，開著的 App 不中斷、log 沒有「協定版本不符」、舊版資料夾正確清除

### 1. 正式安裝程式（Inno Setup，`installer/zootype.iss`）
- [ ] 安裝位置固定 `{autopf}\ZCT`（沿用現在的路徑，開發版與正式版可互相覆蓋）
- [ ] 檔案：zct_tsf.dll、zct-service.exe、zct-settings.exe、dict/*、授權檔
- [ ] 安裝：先結束 zct-service → 複製 → regsvr32（註冊 TSF、設為中文預設）→ 以原本使用者身分加入中文（台灣）鍵盤清單第一位 → 登入自動啟動 → 開始功能表「動物園輸入法 設定」
- [ ] 升級：裝到新的版本資料夾（與 install-dev.ps1 相同邏輯），不覆蓋被載入中的檔案
- [ ] 解除安裝：結束服務 → regsvr32 /u → 從鍵盤清單移除 → 刪檔；使用者資料（%APPDATA%\zct）保留
- [ ] 授權聲明：本專案 MIT/Apache-2.0；libchewing-data 詞庫 LGPL-2.1、語言模型 CC-BY-4.0；CLDR Emoji Unicode-3.0；dwyl 英文單字 Unlicense
- [ ] install-dev.ps1 保留給開發用；安裝邏輯盡量共用（避免兩份行為不同）

### 2. CI（`.github/workflows/ci.yml`，push / PR）
- [ ] windows-latest：fetch-dict.sh → fmt 檢查 → clippy -D warnings → test → build release
- [ ] 快取 cargo 與詞庫資料

### 3. Release（`.github/workflows/release.yml`，推送 v* 標籤）
- [ ] 檢查標籤版本 = Cargo.toml workspace 版本（不一致就失敗）
- [ ] 建置 + 測試 → iscc 產生 `zootype-<版本>-windows-x64-setup.exe` → SHA256
- [ ] 用 gh CLI 建立 GitHub Release，附上安裝檔與 SHA256，自動產生變更說明
- [ ] 未做程式碼簽章：下載時 SmartScreen 會警告（README 說明；之後可考慮簽章）

### 4. macOS（需要先完成 M6）
- [ ] M6：InputMethodKit 前端（Swift）+ cbindgen C ABI 接 zct-core
- [ ] release.yml 加 macos job：.app 打包成 .pkg（安裝到 ~/Library/Input Methods）
- [ ] 簽章 / 公證需要 Apple Developer 帳號（未公證的 pkg 會被 Gatekeeper 擋，需手動允許）

### 5. 自動更新
- [ ] 服務程序每天檢查 GitHub Releases（剛開機延後幾分鐘）；pre-release 預設略過
- [ ] 輸入法圖示右鍵選單「更新到 vX」；下載後驗證 SHA256；靜默安裝（一次 UAC）
- [ ] 開發版（install-dev.ps1）標成 -dev，不自動更新
- [ ] 設定程式「自動檢查更新」開關（預設開）
- [ ] `scripts/release.ps1 <版本>`：改版號、commit、打標籤

### 6. 驗證
- [ ] 本機用 iscc 產生安裝檔，實際安裝 / 升級 / 解除安裝各一次
- [ ] 推到 GitHub 後先推一個測試標籤（例如 v0.2.0-rc.1）確認 release 流程

## M9 選取文字加入詞庫、重新選字（規劃）
- [ ] DLL 讀取 App 中被選取的文字（TSF edit session；不支援的 App 退回剪貼簿並還原）
- [ ] 讀音反查（與詞庫頁自動填注音共用），候選窗列出可能讀音確認後加入並設為優先（預設 `ctrl+shift+a`）
- [ ] 重新選字：選取已送出的文字 → 變回組字狀態重新選字（TSF reconversion），共用上面兩項

## 之後
- M6 macOS 前端（使用者決定延後）
- M8 AI（T0 → T1 → T2）、選字準確率評測
