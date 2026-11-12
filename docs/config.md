# 設定檔

設定檔放在**資料根目錄**，Windows 預設是 `%APPDATA%\ZooType\`（初次安裝時可以選別的位置，之後更新會沿用）。

之後要換位置：設定程式「同步」頁的「資料位置 → 變更位置…」。選空的資料夾會把目前的資料複製過去；選已經有資料的資料夾（例如重灌前放在 D 槽的）會直接改用，原本位置的資料保留。

要放到自訂資料夾（例如要用 git 同步），在 `%APPDATA%\ZooType\location.toml` 寫：

```toml
data_root = "D:/Sync/zct"
```

- 設定檔改完後，**切換到其他 App 再切回來**（或重開 App）就會生效；`[theme]` 1 秒內生效
- 設定程式存檔時會重寫整個檔案，手寫的註解不會保留
- 設定檔寫錯時，輸入法照常運作，改用上一份正確的設定；錯誤會記在 `local/logs/service.log`

## `config.toml`

所有欄位都可以省略，省略的欄位使用預設值。

```toml
[input]
default_mode = "chinese"   # 新開啟的輸入欄位預設中文 / 英文："chinese" | "english"
punctuation = "full"       # 中文模式的標點："full" 全形 | "half" 半形
output = "traditional"     # 送出的字："traditional" 繁體 | "simplified" 簡體
on_focus = "remember"      # 切回 App 時："remember" 保留上次中英狀態 | "reset" 重設為預設

[toggle]
shift = "both"             # 哪個 Shift 切換中英："both" | "left" | "right" | "off"
shift_tap_max_ms = 300     # 按住 Shift 超過這個時間才放開，不算切換

[hotkeys]                  # 留空字串代表不使用
width = "shift+space"      # 切換全形 / 半形
output = ""                # 切換繁體 / 簡體（預設不綁定）
game_mode = "ctrl+shift+f12"  # 開關遊戲模式（所有 App 一起）
forget = "shift+delete"    # 候選窗中：忘記選擇框所在詞的學習紀錄（並取消固頂）
pin = "shift+enter"        # 候選窗中：固頂選擇框所在的詞並選它
emoji = "ctrl+shift+;"     # Emoji：以組字內容當關鍵字搜尋；沒有組字時列出常用 Emoji
ai = "ctrl+shift+'"         # AI 修正組字內容中的同音錯字
composition_bar = "ctrl+shift+f11"  # 切換目前 App 的組字提示列（自動寫進 apps.toml 記住）

[detect]                   # 自動偵測「不在輸入狀態」，見下方說明
legacy_fullscreen = true   # 舊式程式 + 沒有游標 + 無邊框全螢幕（DirectX 遊戲）
exclusive_fullscreen = true  # 有程式獨佔全螢幕

[mixed]                    # 中英混打（組字中單按 Shift 切英文不會送出，英文與數字會放進組字區）
auto_english = true        # 按空白時，剛打的按鍵是英文單字且不是合法注音 → 放進組字區當英文（例：hello）
english_candidate = true   # 剛完成的音節也是英文單字時，候選窗附上英文（例：fu → 七 / fu）
                           # 以上兩項只在組字開頭，或「中文 + 空白」之後判斷（「我 love」會、「我love」不會）
                           # 單字表：一般英文字典 + 內建技術用語（repo、config、api…）+ 自訂的 dict/english.txt（一行一個）
auto_space = false         # 中英文之間自動加空格（盤古之白）

[sync]                     # git 同步，見下方說明
enabled = false
remote = ""                # 例：git@github.com:you/zoo-type-data.git（請用私人 repo）
interval_minutes = 10

[ai]                       # AI 同音字修正（OpenAI 相容 API）；API Key 在設定程式輸入，存在 Windows 認證管理員
enabled = false
endpoint = "http://localhost:11434/v1"   # Ollama；LM Studio 為 http://localhost:1234/v1
model = ""                 # 例：qwen3.5:2b
timeout_seconds = 15

[theme]                    # 候選窗外觀，改完 1 秒內生效
layout = "vertical"        # "vertical" 直式 | "horizontal" 橫式
font = "Microsoft JhengHei UI"
font_size = 16.0           # 8–48
background = "#FFFFFF"
text = "#1A1A1A"
number = "#7A7A7A"
highlight = "#CCE4F7"      # 選擇框
composition_bar = false    # 組字提示列的全域預設（游標上方另開視窗顯示組字內容與游標）

[candidates]
per_page = 9               # 每頁候選字數（1–10）
sort_by_frequency = true   # 候選字依常用程度與個人學習排序

# 自訂標點：覆寫內建的對照，鍵名寫法同快捷鍵
[punctuation.full]
"shift+," = "《"
"shift+." = "》"

[punctuation.half]
"'" = "'"
```

### 快捷鍵寫法

`修飾鍵+主鍵`，不分大小寫：

- 修飾鍵：`ctrl`、`alt`、`shift`、`win`（macOS 上是 `cmd`）
- 主鍵：字母、數字、符號（例如 `,`）、`space`、`enter`、`tab`、`esc`、`f1`–`f24`、方向鍵（`left` 等）

範例：`ctrl+shift+f12`、`shift+space`、`ctrl+alt+s`。

修飾鍵必須完全一致：設定 `shift+space` 時，按 `ctrl+shift+space` 不會觸發。

**沒有綁定的 Ctrl / Alt / Win 組合鍵一律交給 App**，所以 VS Code 的 `Ctrl+,` 這類快捷鍵不會被輸入法攔截。設定繁簡切換時，避免使用 App 常用的組合，例如 `Ctrl+Shift+F` 是 VS Code 的全域搜尋。

## `apps.toml`

依 App 覆寫設定。同一筆規則可以同時寫 Windows 執行檔名和 macOS Bundle ID，兩個平台共用一份檔案。

```toml
[[app]]
name = "VS Code"
windows = "Code.exe"
macos = "com.microsoft.VSCode"
default_mode = "english"
punctuation = "half"

[[app]]
name = "Windows Terminal"
windows = "WindowsTerminal.exe"
default_mode = "english"
on_focus = "reset"

[[app]]
name = "Elden Ring"
windows = "eldenring.exe"
game_mode = "english"
```

| 欄位 | 說明 |
|------|------|
| `name` | 顯示用名稱 |
| `windows` | 執行檔名稱，不分大小寫 |
| `macos` | Bundle ID |
| `default_mode` | 同 `[input]` |
| `punctuation` | 同 `[input]` |
| `output` | 同 `[input]` |
| `on_focus` | 同 `[input]` |
| `composition_bar` | 組字提示列：`true` / `false`；沒寫時純終端機（Windows Terminal、conhost、wezterm…）開啟、其他沿用 `[theme]`。在 App 中按組字提示列快捷鍵會自動寫入 |
| `detect` | `false`：這個 App 不做自動偵測（見下方說明；例如遊戲聊天框是自己畫的，被誤判成不能打字） |
| `game_mode` | `"off"` 不啟用；`"english"` 鎖定英文，按鍵全部交給 App；`"keep"` 保留中英狀態但 Shift 不切換（例如在遊戲聊天室打中文） |

App 的執行檔名稱可以在工作管理員的「詳細資料」分頁查到。

## 自動偵測「不在輸入狀態」

偵測到時，按鍵全部直接交給 App，**Shift 也不會切換中英**。

| 偵測 | 情況 | 能否關閉 |
|------|------|---------|
| 系統訊號 | 焦點不在輸入欄位（桌面、按鈕、清單）、App 主動停用輸入法（很多遊戲會這樣做） | 不能，這是系統明確告知的狀態 |
| `legacy_fullscreen` | 不支援 TSF 的舊式程式 + 畫面上沒有系統游標 + 視窗蓋滿整個螢幕：無邊框全螢幕的 DirectX 遊戲 | 可以 |
| `exclusive_fullscreen` | 有程式獨佔全螢幕：老式全螢幕遊戲 | 可以 |

偵測不到的情況：視窗模式的遊戲，而且沒有停用輸入法。這時請用 `apps.toml` 的 `game_mode` 或遊戲模式快捷鍵。

如果某個 App 明明在輸入框卻打不了中文，在 `apps.toml` 對它設 `detect = false`。

## 學習紀錄

| 檔案（資料根目錄下） | 內容 |
|------|------|
| `local/learning/user_dict.csv` | 自己加的詞與固頂（純文字） |
| `local/learning/history_dict.bin` | 自動學習的選字紀錄（會隨時間衰減） |
| `local/learning/stats.tsv` | 每日統計：日期、送出字數、修正次數 |
| `local/backup/<日期>/` | 每日自動備份，保留最近 7 天 |

- 所有 App 共用同一份學習紀錄；每 30 秒與 App 關閉時存檔
- 存檔先寫暫存檔再改名，寫到一半當機也不會弄壞檔案
- **學習紀錄變亂時**：關閉所有 App（或登出），把 `local/backup/` 裡某一天的檔案複製回 `local/learning/`
- 檔案損壞時，輸入法會改用空的紀錄繼續運作，原檔另存為 `*.broken`

## git 同步

資料根目錄本身就是 git repo。需要先安裝 Git，並設定好遠端的 SSH 金鑰或憑證（沿用你既有的 git 設定）。

| 同步 | 不同步 |
|------|--------|
| `config.toml`、`apps.toml` | `local/`（學習紀錄、備份、log） |
| `dict/user.tsv`（自訂詞、固頂、封鎖） | |
| `stats/<電腦名稱>.tsv`（每日統計） | |

- 兩台電腦改了同一個詞時，以較新的修改為準
- 在設定程式詞庫頁「移除」的詞，其他電腦同步後也會移除
- **新電腦第一次同步時會採用遠端的設定與 App 規則**；本機的自訂詞會合併進去
- 設定檔兩邊同時修改而衝突時，同步會中止並在設定程式的「同步」頁顯示錯誤，需手動處理
- **自動學習紀錄不同步**：libchewing 的學習紀錄是二進位格式、沒有可逐筆合併的介面
- 同步結果記在 `local/sync-status.txt`

## 遊戲模式

| 方式 | 範圍 |
|------|------|
| `apps.toml` 的 `game_mode` | 只影響該 App |
| 快捷鍵 `ctrl+shift+f12` | 所有 App 一起開關，行為等同 `"english"` |

遊戲模式下，Shift 不會切換中英，所有組合鍵都交給 App。開關時游標旁會顯示「遊戲模式」或「一般模式」。
