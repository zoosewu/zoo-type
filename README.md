# ZCT

跨平台（Windows / macOS）注音輸入法，Rust 核心。開發中。

- 需求：[docs/requirements.md](docs/requirements.md)
- 架構：[docs/architecture.md](docs/architecture.md)
- 技術查證：[docs/research.md](docs/research.md)

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
