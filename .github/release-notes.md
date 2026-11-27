## 下載

| 平台 | 檔案 |
|---|---|
| Windows 10 / 11（64 位元） | [zoo-type-windows-x64-setup.exe](https://github.com/zoosewu/zoo-type/releases/latest/download/zoo-type-windows-x64-setup.exe) |
| macOS 15 以上（Apple Silicon） | [zoo-type-macos-arm64.zip](https://github.com/zoosewu/zoo-type/releases/latest/download/zoo-type-macos-arm64.zip) |
| macOS 15 以上（Intel） | [zoo-type-macos-x64.zip](https://github.com/zoosewu/zoo-type/releases/latest/download/zoo-type-macos-x64.zip) |

檔名含版號的同一份檔案是給 0.1.10 以前的版本自動更新用的，手動下載選上面任一個即可。

### Windows
執行安裝檔（需要系統管理員權限）。安裝檔沒有程式碼簽章，Windows 可能顯示「Windows 已保護您的電腦」：按「其他資訊」→「仍要執行」。

### macOS
解壓後雙擊「安裝.command」（或在終端機執行 `sh install.sh`，不需要 sudo），再到系統設定 → 鍵盤 → 輸入方式加入「動物園輸入法」。詳細步驟與 Apple 發行簽章狀態見壓縮包內的 README.txt。

`-tools.tar.gz` 是命令列工具包（模擬打字與評測）；`ngram.bin` 是安裝檔內含的智慧選字模型（CC BY-SA 4.0，由中文維基百科字頻統計而成），供建置與評測使用。一般使用者不需下載。

所有檔案都附 SHA-256 校驗檔。
