# ZCT 開發計畫

## M0 專案骨架
- [x] git init、`.gitignore`、MIT / Apache-2.0 授權檔、README
- [x] Cargo workspace：`zct-core`
- [x] 確認 `chewing` crate（0.14 alpha）API 與系統詞庫載入方式

## M1 核心（純 Rust，可單元測試）
- [x] 資料根目錄解析（`ZCT_HOME` → 指標檔 → 預設）
- [x] `config.toml` 讀取（預設值、中英預設狀態、Shift 設定）
- [x] `Session`：包裝 chewing Editor，注音 → 組字 → 候選 → 送出
- [x] 按鍵放行規則：Ctrl / Alt / Cmd 一律放行（P3）
- [x] Shift 單按切換狀態機（含門檻時間、中途按其他鍵取消）
- [x] 英文模式直接放行
- [x] CLI 測試工具：在終端機模擬按鍵序列，印出組字 / 候選 / 送出結果
- [x] 單元測試 + `process_key` 效能基準

## M2 Windows 最小可用
- [x] `zct-ipc`：訊息格式 + pipe 用戶端 / 伺服器
- [x] `zct-service`：session 管理
- [x] `zct-tsf`：TSF DLL（註冊、按鍵 sink、composition、commit）
- [x] 候選窗（Direct2D，常駐、不搶焦點）
- [x] 安裝 / 解除安裝腳本（`scripts/install-dev.ps1`、`uninstall-dev.ps1`）
- [x] 實機驗證：記事本、VS Code `Ctrl+,`、Windows Terminal、Claude Code（使用者回報行為正常）
- [x] 使用者回饋：候選窗縮小 + 直式 + 選擇框（↑↓ 移動、Enter 選字）；選字後游標跳到詞後
- [ ] x86 / ARM64 DLL 建置

## M3 設定與模式（分支 `m3`）
- [x] 快捷鍵解析（`ctrl+shift+f12` 這類字串）與 F1–F12 按鍵
- [x] `config.toml` 擴充：`[hotkeys]`、`output`（繁 / 簡）、`on_focus`、自訂標點表
- [x] `apps.toml` App 規則表：預設中英、標點寬度、輸出、遊戲模式、切回時重設 / 記住
- [x] 遊戲模式：App 規則 + 手動快捷鍵（全域）；Shift 不切換、組合鍵全放行
- [x] 簡體輸出（ferrous-opencc `Tw2sp`），組字區與候選字也顯示簡體
- [x] 自訂標點對照表（全形 / 半形各一張，覆寫預設）
- [x] 數字鍵盤：一律輸出半形數字；選字時可當數字鍵選字
- [x] Caps Lock：開啟時字母直接輸出英文（與微軟新注音相同）
- [x] 服務程序：新連線時若設定檔有變更就重新載入
- [x] 自動偵測「不在輸入狀態」（使用者要求三層都做）
  - ① 系統訊號：無焦點文件 / 無 context / KEYBOARD_DISABLED / EMPTYCONTEXT（查證微軟 SampleIME；修正其覆寫 bug）
  - ② 舊式程式 + 無系統游標 + 視窗蓋滿螢幕（加上「蓋滿螢幕」避免 IntelliJ 等 Java / Qt 程式誤判）
  - ③ 獨佔全螢幕（SHQueryUserNotificationState，快取 500ms）
  - 可在 `[detect]` 與 App 規則 `detect = false` 關閉 ②③
  - 協定版本 → 4（Hello 回應改為 `Welcome`，帶偵測設定）

## M4 學習機制與詞庫（分支 `m4`）— 對應 P1、P2
- [ ] `Engine`：系統詞庫只載入一次；所有 App 共用同一份使用者詞庫與學習資料
      （目前每個連線各自載入、各自寫檔，多個 App 的學習會互相覆蓋 → P1 的可能成因）
- [ ] 背景存檔：送出時不再同步寫檔（原本約 1ms），每 30 秒與結束時存檔，寫暫存檔再改名（不會寫壞）
- [ ] 每日自動備份，保留 7 份
- [ ] 候選字依頻率排序（語言模型 + 個人學習），預設開啟（P2）
- [ ] 候選窗快捷鍵：Shift+Delete 忘記該詞的學習紀錄、Shift+Enter 固頂（可設定）
- [ ] 快速加詞：組字區 Shift+← / → 選範圍再按 Enter（chewing 內建），顯示「已加入」提示
- [ ] 首選正確率統計（每日），寫入 `local/stats.toml`（4.14）
- [ ] 從 M2 / M3 的舊位置（`local/chewing/v4`）遷移學習資料

> 原規劃的 SQLite：chewing 的學習資料已有半衰期衰減與修剪，使用者詞庫是純文字 CSV。
> 直接沿用、外面包一層管理即可，不另建 SQLite（Simplicity First）。同步用的文字快照留到 M7。

## 之後（依需求文件 MoSCoW 排序）
- M5 Slint 設定 GUI、候選窗主題
- M6 macOS 前端
- M7 git 同步、Emoji 面板、中英混打（B + C + E）
- M8 AI（T0 → T1 → T2）、選字準確率評測

## Review

### M0 + M1（2026-11-05）
- `chewing` 鎖定 `=0.14.0-alpha.4`；詞庫用 libchewing-data v2026.10.27（`scripts/fetch-dict.sh`）
- 測試：29 個全過（單元 16 + 整合 13）；clippy 無警告
- 效能（release）：每次按鍵事件平均約 16µs；建立 session 約 9ms
- 發現，留待後續：
  - Enter 送出約 1ms：chewing 送出時同步寫入學習資料 → M4 移到背景
  - 單字候選排序是靜態的（例如「ㄑㄧˋ」第一個是「企」）→ M4 評估 `sort_candidates_by_frequency` 與自訂排序（P2）
  - 每個 session 各自載入詞庫（約 9ms、數 MB）→ M2 服務程序改為共用詞庫
  - Shift+, 輸出「，」進入組字區（與微軟新注音一致）

### M2 程式碼（2026-11-05，實機驗證前）
- IPC：named pipe 往返平均約 10µs；逾時測試通過（50ms 逾時在 200ms 內返回）
- 服務程序：以模擬用戶端驅動，組字 / 候選窗 / 送出 / Shift 切換正確；候選窗截圖確認繪製正常
- 修正：候選窗寬度計算少算結尾（頁碼被截掉）
- 已知未做：數字鍵盤、Caps Lock、顯示屬性（組字底線）、輸入模式圖示、開始選單 UILess 模式

### M3（2026-11-05）
- 測試：68 個全過；clippy 無警告
- 簡體轉換：載入約 0.2ms、一句約 2µs（release）
- 協定版本 1 → 3（新增 `Focus`；版本 2 已在 M2 commit 中，非本次修改）
- 預設快捷鍵：全半形 `shift+space`、遊戲模式 `ctrl+shift+f12`、繁簡不綁定（避免搶 VS Code 的 Ctrl+Shift+F）
- 自訂標點與 Caps Lock / 數字鍵盤：組字中先送出組字再輸出，與半形標點行為一致
