# ZCT 架構設計

> 對應需求：[requirements.md](requirements.md)　查證：[research.md](research.md)

## 1. 程序模型

```
Windows
┌──────────── App 程序 ────────────┐        ┌──────────── zct-service.exe（每位使用者一個）────────────┐
│ zct_tsf.dll（薄層）               │  pipe  │ 引擎（chewing Editor，每個輸入 context 一個 session）     │
│  - 收按鍵、本地快速放行判斷        │◀──────▶│ 輸入狀態（中 / 英、遊戲模式、App 規則）                    │
│  - composition / commit           │  同步   │ 候選窗 / 提示窗繪製（Direct2D）                           │
│  - GetTextExt 回報游標座標        │  逾時   │ 學習資料（SQLite）、設定熱重載、git 同步（背景）、AI（背景）│
└──────────────────────────────────┘        └───────────────────────────────────────────────────────────┘

macOS：ZCT.app（IMK 程序）直接內嵌 zct-core，不需要 IPC
設定 GUI：zct-settings（Slint），只讀寫資料根目錄的檔案，服務程序監看檔案變更後熱重載
```

## 2. Crate 切分（Cargo workspace）

| crate | 類型 | 平台 | 職責 |
|-------|------|------|------|
| `zct-core` | lib | 全部 | 引擎包裝、輸入狀態機、Shift 判定、按鍵放行規則、設定、資料路徑、簡繁輸出 |
| `zct-ipc` | lib | Windows | DLL ↔ 服務程序的訊息格式與編解碼（固定長度二進位，無配置） |
| `zct-service` | bin | Windows | 常駐服務：session 管理、候選窗繪製、系統匣 |
| `zct-tsf` | cdylib | Windows | TSF 文字服務（x86 / x64 / ARM64） |
| `zct-ffi` | staticlib | macOS | C ABI（cbindgen）給 Swift 前端 |
| `zct-settings` | bin | 全部 | Slint 設定 GUI |

**原則**：所有「判斷」都在 `zct-core`（可單元測試、兩平台共用）；平台層只做轉譯。

## 3. 核心介面（`zct-core`）

```rust
// 平台無關的按鍵事件
pub struct KeyEvent { pub key: Key, pub mods: Modifiers, pub kind: KeyKind /* Down | Up */ }

// 一次按鍵處理的結果，平台層照著做
pub struct KeyResult {
    pub eaten: bool,              // false → 交還給 App
    pub commit: Option<String>,   // 要送出的文字（已做簡繁轉換）
    pub preedit: Preedit,         // 組字區內容 + 游標位置
    pub candidates: Option<CandidatePage>,
    pub mode_changed: Option<InputMode>, // 觸發「中 / A」提示
}

pub struct Session { /* 每個輸入 context 一個 */ }
impl Session {
    pub fn test_key(&self, ev: &KeyEvent) -> bool;       // 不改狀態，只判斷是否會吃掉
    pub fn process_key(&mut self, ev: &KeyEvent) -> KeyResult;
    pub fn reset(&mut self);                              // 失焦 / 切 App
}
```

- `test_key` 讓 TSF 的 `OnTestKeyDown` 不必改變狀態
- **放行規則（P3）**：有 Ctrl / Alt / Cmd → 一律 `eaten = false`（除非該組合是使用者明確綁定的快捷鍵）
- **Shift 判定**：狀態機 `Idle → ShiftDown(t0) → (其他鍵按下 → Cancelled) → ShiftUp`，`ShiftUp` 時未取消且 `now - t0 < 門檻` 才切換

## 4. IPC 協定（`zct-ipc`）

- 傳輸：每個 App 執行緒一條 named pipe 連線 `\\.\pipe\zct-<SID>`，訊息模式
- 安全：SDDL 允許 SYSTEM、使用者本人、`ALL APPLICATION PACKAGES`，Low integrity（參考 Weasel）
- 逾時：DLL 端每次往返 **100ms** 上限；逾時 → 放行、標記斷線、背景重連
- 訊息：`Request { session_id, op: TestKey | ProcessKey | Focus | Blur | CaretRect }`；`Response { eaten, commit, preedit, ... }`
- 候選窗由服務程序繪製，DLL 只回報游標矩形，DLL 不畫任何 UI

## 5. 資料根目錄

```
指標檔（位置固定）：
  Windows  %APPDATA%\zct\location.toml
  macOS    ~/Library/Application Support/zct/location.toml
  內容：data_root = "D:/MyData/zct"
覆寫：環境變數 ZCT_HOME
未設定：指標檔所在資料夾即為資料根目錄

<data_root>/
├─ config.toml      ├─ apps.toml       ├─ dict/       ├─ learning/
├─ themes/          └─ local/（不同步：machine.toml、cache.sqlite、models/、logs/）
```

設定（`config.toml`、`apps.toml`）放在使用者自選的資料根目錄 → 滿足「config 自訂位置」。

## 6. 執行緒與延遲

- 服務程序：每條 pipe 連線一個 worker；引擎呼叫在 worker 內同步完成
- 候選窗更新送到 UI 執行緒，不等待繪製完成就回應 DLL
- AI、git、檔案監看、SQLite 寫入：背景執行緒，透過 channel 回傳，不碰熱路徑
- 量測點：`process_key` 耗時（criterion 基準）、pipe 往返耗時（log 取樣）
