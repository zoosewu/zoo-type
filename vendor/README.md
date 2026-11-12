# vendor

## chewing

[chewing 0.14.0-alpha.4](https://crates.io/crates/chewing/0.14.0-alpha.4)（LGPL-2.1-or-later，授權見 `chewing/COPYING`），只有一處修改：

- `src/user/dict.rs`：加入 `UserDict::remove`，從 [0.14.0-rc.1](https://crates.io/crates/chewing/0.14.0-rc.1) 原樣移植。設定程式詞庫頁的「移除」需要它。

為什麼不直接升級 rc.1：rc.1 改了無效音節與使用者詞庫的行為，輸入法的測試有 5 個失敗，要另外處理。升級後刪掉這個資料夾和根目錄 `Cargo.toml` 的 `[patch.crates-io]`。
