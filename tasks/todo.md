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
- [ ] `zct-ipc`：訊息格式 + pipe 用戶端 / 伺服器
- [ ] `zct-service`：session 管理
- [ ] `zct-tsf`：TSF DLL（註冊、按鍵 sink、composition、commit）
- [ ] 候選窗（Direct2D，常駐、不搶焦點）
- [ ] 實機驗證：記事本、VS Code `Ctrl+,`、Windows Terminal、Claude Code

## 之後（依需求文件 MoSCoW 排序）
- M3 App 規則表、遊戲模式、全半形標點、簡體輸出
- M4 使用者詞庫、快速加詞 / 刪詞、學習資料（SQLite、衰減、快照）
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
