# ZCT

跨平台（Windows / macOS）注音輸入法，Rust 核心。開發中。

- 需求：[docs/requirements.md](docs/requirements.md)
- 架構：[docs/architecture.md](docs/architecture.md)
- 技術查證：[docs/research.md](docs/research.md)
- 設定檔：[docs/config.md](docs/config.md)

## 使用方式

| 操作 | 按鍵 |
|------|------|
| 切換中 / 英 | 單按 Shift |
| 切換標點全形 / 半形 | Shift + 空白鍵 |
| 切換繁體 / 簡體 | 預設不綁定，見 [設定檔](docs/config.md) |
| 開關遊戲模式 | Ctrl + Shift + F12 |
| Caps Lock 開啟 | 字母直接輸出英文 |
| 數字鍵盤 | 一律輸出半形數字；候選窗開啟時可選字 |
| 開啟候選窗 | 組字後按空白（游標前移可選其他位置的字） |
| 移動選擇框 / 翻頁 | ↑ ↓ / ← → |
| 選字 | Enter（選擇框）或數字鍵 |
| 固頂（以後都排第一） | 候選窗中 Shift + Enter |
| 忘記學習紀錄 | 候選窗中 Shift + Delete |
| 加詞 | 組字區 Shift + ← / → 選範圍，再按 Enter |
| 送出 | Enter |
| 取消 | Esc |

### 標點符號（中文模式）

`, . / ; -` 不按 Shift 時是注音（ㄝ ㄡ ㄥ ㄤ ㄦ），所以標點多半要按 Shift。

| 按鍵 | 全形（預設） | 半形 |
|------|------|------|
| Shift + ; | ： | : |
| Shift + ' | ； | " |
| ' | 、 | ' |
| Shift + , | ， | < |
| Shift + . | 。 | > |
| Shift + / | ？ | ? |
| [ / ] | 「 / 」 | [ / ] |
| Shift + [ / ] | 『 / 』 | { / } |
| Shift + 1 | ！ | ! |
| Shift + 9 / 0 | （ / ） | ( / ) |
| Shift + - | — | _ |
| Shift + \ | ｜ | \| |
| Shift + ` | ～ | ~ |
| ` | 符號選單（…、※、括號、希臘字母、數學符號等） | 同左 |

`/`、`;` 等半形符號也可以直接切到英文模式（單按 Shift）輸入。

## 開發

```sh
sh scripts/fetch-dict.sh          # 下載 libchewing 詞庫到 assets/dict
cargo test                        # 執行測試
cargo run -p zct-core --release --example sim -- "wu0 fu4" "<enter>"   # 模擬打字
```

## 授權

程式碼採用 [MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE) 雙授權，任選其一。

第三方元件：
- [libchewing](https://codeberg.org/chewing/libchewing)：LGPL-2.1-or-later
- libchewing 詞庫：LGPL-2.1-or-later；語言模型：CC-BY-4.0
