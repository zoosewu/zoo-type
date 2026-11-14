# ZCT 開發計畫

已完成 M0～M8.3（Windows 版日常可用），細節與檢討見 [archive.md](archive.md)。

## 零散待辦
- [-] x86 / ARM64 DLL 建置：不做，Windows 只出 64 位元（使用者決定 2026-11-13）
- [ ] 實測：設定程式本身能用 ZCT 打中文（Slint 輸入法支援）（原 M5 設定程式與候選窗主題）
- [x] 服務程序還沒啟動時，最前面幾個按鍵會漏給 App：DLL 啟動服務程序後在同一次按鍵裡等它就緒（最多 1.5 秒）（原 M8.2：打字回饋修正）
- [ ] 設定程式詞庫頁加上英文單字清單（目前可用 Shift+Delete 或直接編輯 dict/english.txt）（原 M8.2：打字回饋修正）
- [ ] 手動遊戲模式時 DLL 直接放行、不經 IPC（現在每鍵仍問服務程序，正常 <1ms，服務卡住最多 100ms）（原 M8.2：打字回饋修正）
- [ ] tsf_probe 會把快捷鍵切換的結果寫進使用者真正的 `apps.toml` → 改用獨立的資料資料夾（`ZCT_HOME`）
- [x] 升級 chewing 0.14.0-rc.1（2026-11-13）：打錯的注音由 chewing 保留（含聲調）；注音打到一半按 Enter 連同注音送出；一般自訂詞加權 10 → 0（讀檔時轉換）
- [ ] 實測「右 Alt + 注音鍵」：TSF 是否把 Alt 組合鍵交給輸入法、放開 Alt 會不會讓 App 選單取得焦點

## M10 Release 與 GitHub Actions（repo：zoo-type；分支 `m10`；初版完成前不發 release）

前置：已完成（M8.3 合併回 main）。按鍵診斷保留（`local/debug-keys` 存在才記錄非字元鍵）

### 0. 新舊版本並存（取代協定向下相容；使用者決定 2026-11-08）
- [x] 每個版本裝在 `Program Files\ZCT\<版本資料夾>\`，根目錄 `current.txt` 記錄目前版本與協定版本
- [x] 連線名稱與單一實例 mutex 帶協定版本：不同協定的服務程序可以同時存在
- [x] 舊版服務程序：協定相同 → 讓位給新版；協定不同 → 唯讀（不存學習、不寫設定、不同步），沒有連線時結束
- [x] `zct-service --protocol` 印出協定版本（安裝程式寫 current.txt 用）
- [x] install-dev.ps1：裝到 `dev-<日期時間>-<commit>`、不強制結束服務程序；舊版整個資料夾刪除或排到重開機刪除
- [x] 實機驗證（2026-11-09）：連續安裝兩次；開著的 VS Code 照常輸入、舊服務程序幾秒內讓位、log 沒有「協定版本不符」、使用中的舊版排到重開機刪除

### 1. 正式安裝程式（Inno Setup，`installer/zoo-type.iss`）
- [x] 安裝位置固定 `{autopf}\ZCT\<版本>`（與開發版相同的根目錄，可互相覆蓋）
- [x] 檔案：zct_tsf.dll、zct-service.exe、zct-settings.exe、dict/*、授權檔、THIRD-PARTY-NOTICES.txt
- [x] 安裝：複製到版本資料夾 → install.ps1 System（regsvr32、current.txt、清除舊版）→ install.ps1 User 以安裝者本人身分（鍵盤清單第一位、登入自動啟動、開始功能表「動物園輸入法 設定」）；不要求關閉程式
- [x] 升級：裝到新的版本資料夾（與 install-dev.ps1 共用 install.ps1），不覆蓋被載入中的檔案
- [x] 解除安裝：uninstall.ps1（結束服務 → regsvr32 /u → 從鍵盤清單移除 → 刪除或排程刪除）；使用者資料保留
- [x] 授權聲明：installer/THIRD-PARTY-NOTICES.txt（libchewing LGPL 靜態連結 → 原始碼公開即符合）
- [x] Slint 授權：設定程式「關於」頁放 AboutSlint（使用者決定 2026-11-13；已有）
- [x] install-dev.ps1 保留給開發用；安裝邏輯共用 scripts/install.ps1、uninstall.ps1
- [x] 免管理員安裝：不可行（實驗 2026-11-13）。TIP 只寫在 HKCU（COM 類別 + CTF\TIP 語言設定檔）時，
  `InstallLayoutOrTip` 回報成功但語言清單沒有它，而且 TSF 會把 HKCU 的語言設定檔當殘留刪掉；HKCU 只是已在 HKLM 登記的 TIP 的個人設定

### 2. CI（`.github/workflows/ci.yml`，push / PR）
- [x] windows-latest：fetch-dict.sh → fmt 檢查 → clippy -D warnings → test → build release
- [x] 快取 cargo（rust-cache）與詞庫資料（以 fetch-dict.sh 內容為 key）

### 3. Release（`.github/workflows/release.yml`，推送 v* 標籤）
- [x] 檢查標籤版本 = Cargo.toml workspace 版本（不一致就失敗）
- [x] 建置 + 測試 → iscc 產生 `zootype-<版本>-windows-x64-setup.exe` → SHA256
- [x] 用 gh CLI 建立 GitHub Release，附上安裝檔與 SHA256，自動產生變更說明
- [x] 未做程式碼簽章：下載時 SmartScreen 會警告（README 說明；之後可考慮簽章）

### 4. macOS（需要先完成 M6）
- [ ] M6：InputMethodKit 前端（Swift）+ cbindgen C ABI 接 zct-core
- [ ] release.yml 加 macos job：.app 打包成 .pkg（安裝到 ~/Library/Input Methods）
- [ ] 簽章 / 公證需要 Apple Developer 帳號（未公證的 pkg 會被 Gatekeeper 擋，需手動允許）

### 5. 自動更新
設計（使用者確認 2026-11-14）：免 UAC 更新 = 安裝時建立 SYSTEM 排程工作 + 安裝檔數位簽章（ed25519）
- [x] 簽章工具 `zct-release`：`keygen`（產生金鑰，私鑰給 GitHub Secrets `ZOO_TYPE_SIGNING_KEY`）、`sign`
  - 簽的內容：`zoo-type-update-v1` + 版號 + 安裝檔 SHA256；`.sig` 檔記版號與簽章（防止拿舊版的合法安裝檔降級）
- [x] release.yml 用私鑰簽安裝檔，上傳 `.sig`
- [x] `zoo-type-updater.exe`（裝在 Program Files，以 SYSTEM 執行）：把待安裝的檔案複製到受保護的資料夾 → 驗證簽章（內建公鑰）
  與版號比目前新 → `/VERYSILENT` 執行安裝檔；任何一步失敗就不安裝
- [x] 安裝程式建立排程工作「動物園輸入法 更新」：SYSTEM 執行 updater，一般使用者可以啟動（SDDL 給 Authenticated Users 執行權）；解除安裝時移除
- [x] 以 SYSTEM 靜默更新時略過安裝程式的「使用者部分」；服務程序啟動時把登入自動啟動改成目前版本的路徑
- [x] 服務程序（只有正式版）：開機後幾分鐘、之後每天查 GitHub Releases latest（不含 pre-release）；
  有新版就下載安裝檔與 `.sig` 到 `%ProgramData%\ZooType\update`
- [x] 輸入法圖示右鍵選單「更新到 vX」（協定加上更新狀態）→ 服務程序啟動排程工作
- [x] 開發版與正式版分開：zct-core 的 `dev` feature（`cargo build-dev`），CLSID、語言設定檔、名稱、資料 / 安裝資料夾、管道、mutex、認證名稱都不同；install.ps1 / uninstall.ps1 加 `-Channel` 並確認程式是同一套
- [x] 開發版不自動更新
- [x] 設定程式「自動檢查更新」開關（預設開）
- [ ] `scripts/release.ps1 <版本>`：改版號、commit、打標籤

### 6. 驗證
- [x] 本機產生安裝檔並實際安裝（2026-11-09）：資料夾、current.txt、註冊、鍵盤清單、自動啟動、捷徑、解除安裝登記、舊版排程刪除都正確
- [ ] 解除安裝 / 從正式版升級各測一次
- [ ] 推到 GitHub 後先推一個測試標籤（例如 v0.2.0-rc.1）確認 release 流程

## M9 選取文字加入詞庫、重新選字（規劃）
- [ ] DLL 讀取 App 中被選取的文字（TSF edit session；不支援的 App 退回剪貼簿並還原）
- [ ] 讀音反查（與詞庫頁自動填注音共用），候選窗列出可能讀音確認後加入並設為優先（預設 `ctrl+shift+a`）
- [ ] 重新選字：選取已送出的文字 → 變回組字狀態重新選字（TSF reconversion），共用上面兩項

## 之後
- M6 macOS 前端（使用者決定延後）
- M8 AI（T0 → T1 → T2）、選字準確率評測
