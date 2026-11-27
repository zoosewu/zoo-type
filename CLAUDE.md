## 作品集（apex-code）

本專案列在 zoosewu 的作品集：https://zoosewu.github.io/apex-code/
- 截圖：依 https://github.com/zoosewu/apex-code/blob/main/docs/SCREENSHOTS.md 放在 `docs/screenshots/`；
  介面有明顯改變時一併更新截圖與 captions.json
- 要顯示「作者的其他專案」時：依 https://github.com/zoosewu/apex-code/blob/main/docs/API.md 讀取 API，不要把清單寫死在本專案

## 發布、更新與授權（zoo-release: v1）

發布流程、更新機制、相關 UI 與授權依 https://github.com/zoosewu/zoo-release 的規範：
- commit 使用慣例式提交；推 tag `vX.Y.Z` 發布（docs/RELEASE.md）
- 更新檢查、提示與安裝（docs/UPDATE.md），新版本橫幅與「關於」頁（docs/UI.md）
- 授權依 docs/LICENSING.md：產品 AGPL-3.0-only＋商業授權、函式庫與工具 MIT；加入 GPL／AGPL 的相依套件前先問使用者
- 外部 PR 必須勾選同意 CONTRIBUTING.md 的貢獻授權才能合併；不要合併沒有同意的 PR
- 修改發布、更新或「關於」頁前先讀對應的規範，不要自行偏離；規範不適用時回報使用者
- 使用者說「更新 zoo-release 規範」時：讀 docs/ADOPTION.md 的「變更紀錄」，只做從目前版本到最新版本列出的「專案要做的事」，最後把上面標題的版本號改成最新版
