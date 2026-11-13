# 動物園輸入法（zoo-type）

大千排列的繁體注音輸入法，Rust 核心。目前支援 Windows 10 / 11（64 位元），macOS 開發中。

## 安裝

到 [Releases](https://github.com/zoosewu/zoo-type/releases) 下載 `zoo-type-<版本>-windows-x64-setup.exe` 執行（需要系統管理員權限）。

安裝檔沒有程式碼簽章，Windows 可能顯示「Windows 已保護您的電腦」：按「其他資訊」→「仍要執行」。可以用同一頁的 `.sha256` 檔核對下載的檔案。

- 需求：[docs/requirements.md](docs/requirements.md)
- 架構：[docs/architecture.md](docs/architecture.md)
- 技術查證：[docs/research.md](docs/research.md)
- 設定檔：[docs/config.md](docs/config.md)

## 設定

右鍵點工作列右下角的輸入法圖示（中 / 英）→ 設定…：一般、快捷鍵、外觀（即時預覽）、App 規則、詞庫、學習統計與備份還原。
也可以直接編輯設定檔，見 [docs/config.md](docs/config.md)。

## 使用方式

| 操作 | 按鍵 |
|------|------|
| 切換中 / 英 | 單按 Shift（組字中切換不會送出，英文、數字直接加進組字區：第 Shift 6 Shift 章） |
| 切換標點全形 / 半形 | Shift + 空白鍵 |
| 切換繁體 / 簡體 | 預設不綁定，見 [設定檔](docs/config.md) |
| 開關遊戲模式 | Ctrl + Shift + F12 |
| 組字提示列（看不到組字時，例如 VS Code 終端機裡的 Claude Code） | Ctrl + Shift + F11：只切換目前的 App，會記住 |
| Caps Lock | 只影響英文大小寫，不切換中英（中文模式照常打注音） |
| 數字鍵盤 | 半形數字；組字中會加進組字區；候選窗開啟時可選字 |
| 開啟候選窗 | 組字後按 ↓（游標前移可選其他位置的字） |
| 移動選擇框 / 換欄 | ↑ ↓ / ← →（按 ← → 會展開成多欄，所有候選一次看到） |
| 選字 | Enter（選擇框）或數字鍵 |
| 設為優先（以後都排第一） | 候選窗中 Shift + Enter |
| 忘記學習紀錄 | 候選窗中 Shift + Delete |
| 加詞 | 組字區 Shift + ← / → 選範圍，再按 Enter |
| 自訂縮寫 | 設定程式詞庫頁加詞時讀音填英文字母（例如 tw → 台灣）；打 tw 再按空白就換成詞。縮寫剛好是注音（例如 su）時照常打注音，詞附在候選窗；英文模式只在組字中換 |
| Emoji | 候選窗中按 Tab；或打出關鍵字再按 Ctrl + Shift + ; |
| 輸出注音 | 候選窗中按 Tab，選最後的注音選項 |
| 輸入法圖示 | 工作列右下角「中 / 英」：左鍵切換、右鍵開設定、遊戲模式、重新啟動 |
| 中英混打 | 在句首或空白之後打英文單字再按空白（例如 hello），英文會留在組字區，Enter 整句送出；← → 一次跳過一個英文單字 |
| 英文單字學習 | 組字區用 Shift 打的英文，送出後會記住，下次打完按空白就是英文；游標停在英文單字上按 Shift + Delete 取消；也可編輯資料夾中的 dict/english.txt |
| AI 修正錯字 | 組字中按 Ctrl + Shift + '，修正結果出現在候選窗，Enter 採用（需在設定 → AI 啟用） |
| 送出 | Enter |
| 取消 | Esc；注音打到一半按 Backspace 取消整個音節 |

### 標點符號（中文模式）

數字、字母和 `, . / ; -` 不按 Shift 時是注音，所以符號照鍵帽分兩層打：

| 按鍵 | 全形（預設） | 半形 |
|------|------|------|
| `` ` `` 再按 , / . | ， / 。 | , / . |
| `` ` `` 再按 ; / ' / 1 | ； / ＇ / １ | ; / ' / 1 |
| Shift + , / . / / | ＜ / ＞ / ？ | < / > / ? |
| Shift + ; / ' | ： / ＂ | : / " |
| [ / ] | ［ / ］ | [ / ] |
| Shift + 1–0、- = [ ] | 全形版（！＠＃…＿＋｛｝） | 半形版 |

- `` ` `` 連按兩下打出 `` ` `` 本身；也可以設定成按住右 Alt + 鍵
- 想要 Shift + , 出「，」：設定程式「一般 → 次要符號」選「`` ` `` 或 右 Alt + Shift + 鍵」
- 符號選單（…、※、希臘字母）改成快捷鍵，預設不綁定
- 細節見 [docs/config.md](docs/config.md#符號鍵)

## 開發

```sh
sh scripts/fetch-dict.sh          # 下載 libchewing 詞庫到 assets/dict
cargo test                        # 執行測試
cargo run -p zct-core --release --example sim -- "wu0 fu4" "<enter>"   # 模擬打字
```

### 開發版與正式版

開發版「動物園輸入法 開發版」是另一套獨立的輸入法，和正式版同時安裝也互不影響：鍵盤清單裡各有一個項目、
資料各自存在 `%APPDATA%\ZooType` 與 `%APPDATA%\ZooType-dev`，開發版壞了照樣用正式版打字。

```powershell
cargo build-dev                   # 建置開發版（target/dev）
scripts/install-dev.ps1           # 以系統管理員身分安裝到 Program Files\ZooType-dev
scripts/uninstall-dev.ps1         # 移除開發版（資料保留）
```

第一次安裝開發版後，工作列可能把它的圖示顯示成「繁體」（explorer 快取了讀不到圖示的結果），重新啟動 explorer 或重新登入就會顯示「動」加 DEV 標籤。

正式版用 `scripts/build-installer.ps1` 產生安裝程式。

## 授權

程式碼採用 [MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE) 雙授權，任選其一。

第三方元件：
- [libchewing](https://codeberg.org/chewing/libchewing)：LGPL-2.1-or-later
- libchewing 詞庫：LGPL-2.1-or-later；語言模型：CC-BY-4.0
