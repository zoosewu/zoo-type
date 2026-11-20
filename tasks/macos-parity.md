# macOS 與 Windows 功能對齊

## 開發與驗收

- [x] AI：共用 HTTP 修正、非同步回傳、同音字保護、過期結果忽略、取消與失敗提示。
- [x] 憑證：macOS Keychain，正式／開發版分開，儲存／清除／狀態與連線測試。
- [x] 組字提示列快捷鍵：按 App 寫回 apps.toml，保留其他平台規則。
- [x] macOS 安全輸入／全螢幕偵測與 per-App 開關，手動遊戲模式。
- [x] 候選窗滑鼠選字、多欄、捲動翻頁，不搶輸入焦點。
- [x] 輸入法選單：中英、繁簡、遊戲模式、設定、重新載入、更新。
- [x] 設定熱載入：沒有組字時套用變更，保留輸入狀態。
- [x] 自動更新：排程／手動檢查、官方簽章、版本與架構驗證、下載、安裝與還原。
- [x] 發布：macOS zip 數位簽章、Developer ID／notarytool／stapler 整合。
- [x] 修正發布說明與安裝包 README，CI 加入新驗收。
- [x] Rust／Swift 自動化與原生視窗驗收。

Apple Developer 簽章／公證的實際執行需要使用者提供 GitHub Secrets；不將私鑰放入 repo 或聊天。

## 發行驗證

- [x] Apple Silicon 正式版與開發版建置、AppKit 驗收、ZIP 打包。
- [ ] 由 Release CI 在 macOS 15 的 Intel 與 Apple Silicon runner 建置並驗證發布檔。
- [ ] 配置 Apple 憑證後，驗證公證票證及從上一版正式安裝升級。
- [ ] 在 TextEdit、Safari、VS Code、Terminal 逐一驗證新版 IMK 的焦點與組字行為。
