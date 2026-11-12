# 技術查證紀錄

> 查證日期：2026-11-04（Context7 + crates.io / Codeberg / GitHub / 官方文件）
> 「未查證」= 推論或待實測

## 1. 注音引擎：libchewing ✅ 採用

- 主開發在 Codeberg：https://codeberg.org/chewing/libchewing （GitHub 為鏡像）
- 穩定版 0.13.1（2026-07-15）；master 為 0.14.0-alpha.4（API 與詞庫格式大改）；本專案自 2026-11-13 改用 0.14.0-rc.1
- crates.io：`chewing`（純 Rust、`#![deny(unsafe_code)]`），可直接當 Rust crate 使用
- 授權：**LGPL-2.1-or-later** → 開源專案靜態連結可行；修改 libchewing 本身須維持 LGPL
- 能力：
  - 大千等約 15 種排列；0.14 支援自訂按鍵綁定
  - 整句：m-A* 路徑搜尋 + unigram / bigram 靜態語言模型（README：首選整句約 97%）
  - `Editor`：`paginated_candidates()`、`select(n)` 等
  - `UserDict`：`insert` / `remove` / `ban` / `boost` / `deboost`
  - `HistoryDict`：自動學習，含半衰期衰減
  - **`ConversionEngine` 是公開 trait** → 可包裝原引擎，加入自己的重新排序
- 限制：
  - 1.0 前 Rust API 每版可能變動 → 鎖版本
  - 使用者學習的儲存層不是 trait，無法直接換成自己的 SQLite → 排序層合併分數 + 同步回 `UserDict`；必要時小幅 fork
- 詞庫資料授權：
  - `tsi.csv` / `word.csv`：LGPL-2.1-or-later ✅
  - v4 bigram 語言模型（CC-100 訓練）：CC-BY-4.0 ✅（需署名）
  - `dict/moe`（教育部國語辭典簡編本）：CC BY-ND 3.0 TW ❌ 避開
- 工具：`chewing-cli` 可從 CSV / ARPA 建立自訂詞庫
- ⚠️ windows-chewing-tsf 是 **GPL-3.0** → 不抄程式碼

## 2. Windows TSF ✅ 可行

- `windows` crate 0.62.2；`#[implement]` 可實作所有需要的 TSF 介面
- 純 Rust TSF 實證：dec32/ajemi https://github.com/dec32/ajemi
- DLL + 服務程序架構：小狼毫（WeaselTSF ↔ WeaselServer，命名管道同步往返）、PIME、清風 WindInput
- 同步 IPC：每鍵一次往返可接受；**必須設逾時**，逾時放行並標記服務失聯
  - 已知坑：rime/weasel #1487（並發崩潰）、#1473（字元卡住）、#1940（GPU 資源累積）
- **註冊必須寫 HKLM → 安裝需要系統管理員權限**；DLL 放 `Program Files`
  - 提權程序會忽略 HKCU 的 COM 註冊
  - AppContainer（Store App、開始選單搜尋）：檔案需 `ALL APPLICATION PACKAGES` 權限；IPC 管道要設定對應的 SDDL（參考 Weasel `SecurityAttribute.cpp`）
  - 微軟警告：繞過 AppContainer 規則的輸入法可能被當成惡意軟體
- 必要 category：`IMMERSIVESUPPORT`、`SYSTRAYSUPPORT`、`UIELEMENTENABLED` 等
- 開始選單搜尋框強制 UILess → 需實作 `ITfIntegratableCandidateListUIElement`
- 候選窗：owner 用 `ITfContextView::GetWnd`；發送 `EVENT_OBJECT_IME_SHOW/HIDE/CHANGE`；UIA AutomationId `IME_Candidate_Window`
- DLL 需建置 **x86 / x64 / ARM64**
- 游標：`GetTextExt` 可能回 `TS_E_NOLAYOUT` 或 `{0,0,0,0}` → fallback：沿用上次位置 → 前景視窗 + `GetCaretPos` / `GetGUIThreadInfo` → 視窗角落
- Windows Terminal 的 TSF 實作已重寫（microsoft/terminal #13805），精準度待實測
- 組合鍵放行：OnTestKeyDown / OnKeyDown 開頭檢查 Ctrl / Alt，`pfEaten=FALSE`
- Shift 單按：自行在 KeyEventSink 追蹤 down / up（Weasel 做法）
- **全螢幕偵測**：`SHQueryUserNotificationState` 只偵測得到獨佔全螢幕，Win11 多數遊戲是無邊框視窗 → 補充「前景視窗矩形 = 螢幕矩形」判斷（未查證）
- Rust 風險：`windows` crate 版本間常有破壞性變更；所有 COM 回呼需防 panic（`catch_unwind` 或 `panic=abort`）；TSF 重入與借用規則衝突

## 3. Slint ✅ 採用（設定 GUI）

- 1.18.1（2026-09-21）
- 授權三選一：GPLv3 / Royalty-free（須顯示 `AboutSlint` 或 badge）/ 商業
- IME：大致可用；未解 issue #10861（Win11 組字中失焦掉最後一個字）；#8716（femtovg 字型 fallback 卡住）
- 渲染器建議 `renderer-skia`（CJK 品質最穩）
- `ListView` 只建立可見元素，適合大型詞庫表格（效能未實測）
- `SystemTrayIcon` 只在未發佈版文件中 → 系統匣改由服務程序原生實作
- 候選窗預覽與實際候選窗須共用同一份主題資料模型

## 4. macOS InputMethodKit ✅ 可行

- IMKit 仍可用；**主流輸入法都自繪候選窗**（Squirrel `NSPanel`、vChewing 已棄用 IMKCandidates、McBopomofo 自有 CandidateUI）
- Swift 為主；最低支援 macOS 13 為業界常見
- 重新選字：`selectedRange` / `attributedSubstring` 各 App 行為不一 → best-effort + 依 Bundle ID 降級
- 安裝：`~/Library/Input Methods`（McBopomofo）或 `/Library/Input Methods`（Squirrel，需 sudo）
- Developer ID 散佈必須公證 + Hardened Runtime
- **沙盒會限制檔案路徑**（只能用 container / app group）→ 與「自訂資料資料夾」衝突
- 依 App 偵測：`client().bundleIdentifier()`；Squirrel `app_options` 可當參考
- `IsSecureEventInputEnabled()` 是全系統狀態，不代表目前欄位
- Swift ↔ Rust：cbindgen 0.29.4（C ABI、零序列化）或 UniFFI 0.32.2（自動產生、有序列化開銷）

## 5. 周邊函式庫

| 用途 | 選擇 | 版本 | 授權 | 理由 |
|------|------|------|------|------|
| 繁→簡 | `ferrous-opencc` | 0.4.0 | Apache-2.0 | 純 Rust、內建 `Tw2sp` |
| 大模型推論 | `llama-cpp-2` | 0.1.161 | MIT / Apache-2.0 | 活躍、CUDA / Metal、可取 logits |
| T0 小模型 | `tract-onnx` | 0.23.9 | MIT / Apache-2.0 | 純 Rust、無 DLL |
| OpenAI 相容 | `async-openai` | 0.43.0 | MIT | 支援自訂 base URL |
| Git | 系統 `git` 指令 | — | — | `gix` 不支援 push；`git2` 不讀 SSH 設定 |
| 鑰匙圈 | `keyring-core` + 平台 store | 1.0.1 | MIT / Apache-2.0 | 依賴最少 |
| SQLite | `rusqlite`（bundled） | 0.41.1 | MIT | 同步 API、低延遲 |
| Emoji 關鍵字 | Unicode CLDR `annotations/zh-Hant` | CLDR 49 | Unicode-3.0 | 編譯時嵌入 |

排除：`zhconv`（GPL-2.0）、`ort`（仍為 RC）、`llama_cpp`（停止維護）
