# 動物園輸入法 架構設計

> 程式內部代號為 ZCT（crate 名稱 `zct-*`、環境變數 `ZCT_HOME` 等）。使用者看得到的名稱：安裝資料夾 `C:\Program Files\ZooType`、
> 資料資料夾 `%APPDATA%\ZooType`、程式 `zoo_type.dll`、`zoo-type-service.exe`、`zoo-type-settings.exe`。
> 改名前的 `ZCT` / `%APPDATA%\zct` 會在安裝與第一次啟動時自動搬移或清除。

> 對應需求：[requirements.md](requirements.md)　查證：[research.md](research.md)

## 1. 程序模型

```
Windows
┌──────────── App 程序 ────────────┐        ┌──────── zoo-type-service.exe（每位使用者一個）──────────┐
│ zoo_type.dll（薄層）              │  pipe  │ 引擎（chewing Editor，每個輸入 context 一個 session）     │
│  - 收按鍵、本地快速放行判斷        │◀──────▶│ 輸入狀態（中 / 英、遊戲模式、App 規則）                    │
│  - composition / commit           │  同步   │ 候選窗 / 提示窗繪製（Direct2D）                           │
│  - GetTextExt 回報游標座標        │  逾時   │ 學習資料（SQLite）、設定熱重載、git 同步（背景）、AI（背景）│
└──────────────────────────────────┘        └───────────────────────────────────────────────────────────┘

macOS：動物園輸入法.app（IMK 程序）直接內嵌 zct-core，不需要 IPC
設定 GUI：zct-settings（Slint），只讀寫資料根目錄的檔案，服務程序監看檔案變更後熱重載
```

## 2. Crate 切分（Cargo workspace）

| crate | 類型 | 平台 | 職責 |
|-------|------|------|------|
| `zct-core` | lib | 全部 | 引擎包裝、輸入狀態機、Shift 判定、按鍵放行規則、設定、資料路徑、簡繁輸出 |
| `zct-host` | lib | 全部 | 主機層：引擎開啟、存檔備份、git 同步、凍結與恢復、搬移資料；session 生命週期、設定熱載入、AI 背景請求、遊戲模式、組字提示列、中英狀態記憶、提示文字 |
| `zct-ipc` | lib | Windows | DLL ↔ 服務程序的訊息格式與編解碼（固定長度二進位，無配置） |
| `zct-service` | bin | Windows | 常駐服務：session 管理、候選窗繪製、系統匣 |
| `zct-tsf` | cdylib | Windows | TSF 文字服務（x86 / x64 / ARM64） |
| `zct-ffi` | staticlib | macOS | C ABI（cbindgen）給 Swift 前端 |
| `zct-settings` | bin | 全部 | Slint 設定 GUI |

**原則**：所有「判斷」都在 `zct-core`（輸入法本身）與 `zct-host`（主機：一個程序、多個輸入 context 怎麼管理），
兩平台共用、可單元測試；平台層（`zct-service` + `zct-tsf`、`zct-ffi` + Swift）只負責 UI 與 IO：
收按鍵、畫候選窗、讀寫系統憑證、播放提示音、IPC / C ABI 轉換。

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
  Windows  %APPDATA%\ZooType\location.toml
  macOS    ~/Library/Application Support/ZooType/location.toml
  內容：data_root = "D:/MyData/zct"
覆寫：環境變數 ZCT_HOME
未設定：指標檔所在資料夾即為資料根目錄

<data_root>/
├─ config.toml      ├─ apps.toml       ├─ dict/       ├─ learning/
├─ themes/          └─ local/（不同步：machine.toml、cache.sqlite、models/、logs/）
```

設定（`config.toml`、`apps.toml`）放在使用者自選的資料根目錄 → 滿足「config 自訂位置」。

用 `ZCT_HOME` 指定資料位置時，服務程序的連線名稱與單一實例 mutex 會加上由路徑算出的後綴：
一個資料位置一個服務程序。DLL 啟動的服務程序繼承 App 的環境變數，所以 tsf_probe 這類測試程式
設好 `ZCT_HOME` 就會用自己的服務程序與資料，不會碰到使用者正在用的。

## 6. 執行緒與延遲

- 服務程序：每條 pipe 連線一個 worker；引擎呼叫在 worker 內同步完成
- 候選窗更新送到 UI 執行緒，不等待繪製完成就回應 DLL
- AI、git、檔案監看、SQLite 寫入：背景執行緒，透過 channel 回傳，不碰熱路徑
- 量測點：`process_key` 耗時（criterion 基準）、pipe 往返耗時（log 取樣）


## 7. 跨平台設定介面

`zct-preferences` 是 Windows 與 macOS 設定的共用入口。`catalog` 定義所有全域設定與應用程式規則的標題、分頁、型別、預設值、選項及平台能力。完整性測試會比對 `Config` 和 `AppRule` 的序列化欄位，新增欄位時必須同步提供目錄描述。

`SettingsStore` 統一載入、舊格式轉換、驗證與儲存 `Document { config, apps }`。儲存使用檔案鎖與內容版本，拒絕過期視窗覆寫；兩份檔案先完成暫存，再逐一原子替換。這不是兩份檔案跨斷電的原子交易。

`Backend` 以 `Action` / `Reply` 抽象詞庫、讀音、統計、備份、還原、同步與 AI 憑證操作。Windows 設定程式透過 named pipe 轉送引擎操作，Windows 服務與 macOS FFI 共用 `EngineBackend`；憑證與網路工作留在平台實作。macOS Swift `PreferencesClient` 透過 C ABI 的 JSON 請求呼叫共用 `Manager`。UI 不自行解析或寫入 TOML。

macOS 使用 AppKit 九個設定頁；Windows 保留原生 Slint 分頁，增加由同一份目錄產生的「全部設定」。兩邊使用相同設定值、預設值與驗證。Mac 的 AI 修正由 Rust 背景執行緒呼叫共用的 `zct-ai`，IMK 在主執行緒取得結果，金鑰由 Keychain 管理。AppKit 根據安全輸入、前景視窗與文字游標狀態判斷是否放行按鍵。組字提示列快捷鍵透過共用設定層寫入目前 App 的規則。正式版更新使用 `zct-update` 驗證對應架構的 Release ZIP 與簽章，再替換使用者安裝的 .app。

還原會凍結舊引擎；主機須重新開啟引擎，現有輸入 session 須重新啟用，避免舊學習資料覆寫還原結果。


Git 執行流程在 `zct-preferences::git`，供 Windows 背景服務與 macOS `SyncWorker` 共用。macOS 透過共用管理介面的 `SyncNow` 排入背景工作，`SyncStatus` 回報執行中／結果；30 秒儲存排程依設定間隔觸發自動同步。背景工作持有 `Arc<Engine>`，不共享 Swift 或 C ABI handle；每個主機只允許一個同步，另以資料目錄中的檔案鎖避免多程序同時同步。還原與資料搬移必須等待同步完成，避免舊引擎寫回。


## 8. 智慧選字模型的發布

`ngram.bin`（T0 的字 n-gram 模型）只能在有維基百科 dump 與本機 LLM 的電腦上用 `zct-train` 訓練，不放進 repo。

每個正式版 Release 都附上 `ngram.bin` 與 `ngram.bin.sha256`。`scripts/fetch-model.sh` 從最新正式版
（`releases/latest/download/`）下載並比對雜湊；CI 與 Release workflow 每次都執行它（不靠詞庫快取），
Windows 安裝檔與 macOS .app 打包整個 `assets/dict`，Release workflow 再把同一份模型附到新版上，模型就一版一版傳下去。
建置時缺少模型會失敗，不會發布沒有智慧選字的版本。

換模型：
1. `zct-train` 訓練並評測（`zct-eval`）
2. 把新的 `ngram.bin` 與 `ngram.bin.sha256`（`雜湊␠␠ngram.bin`）換到目前最新的正式版 Release 上
3. 之後的 CI 與新版建置就會用新模型

模型授權 CC-BY-SA-4.0，來源與變更說明在 `installer/THIRD-PARTY-NOTICES.txt` 與 `macos/THIRD-PARTY-NOTICES.txt`。

### 組字與候選的顯示介面

`Preedit::display()` 提供游標前文字與目標詞的前、中、後三段。Windows 的 DirectWrite 組字列以這些片段量測字寬；TSF 和 macOS IMK 使用同一份資料轉出的 UTF-16 游標與範圍。AppKit 收到完整組字資料後，以 Core Text 計算螢幕座標。

`CandidatePage` 提供選取列、數字與固頂標記、展開的各欄，以及純提示模式。Windows 候選窗直接取用，macOS 由 C ABI 複製到 Swift 的顯示資料。平台各自管理視窗位置、字型與 App 的行內輸入協定。
