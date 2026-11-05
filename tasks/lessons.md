# Lessons

## PowerShell 腳本含中文必須存成 UTF-8 with BOM（2026-11-05）
- 現象：`install-dev.ps1` 在系統管理員視窗中解析失敗，什麼都沒安裝
- 原因：Windows PowerShell 5.1（`powershell.exe`）讀取沒有 BOM 的 .ps1 時使用系統字碼頁（繁中 Windows 為 Big5），UTF-8 中文被讀成亂碼，破壞字串引號
- 規則：
  - 含非 ASCII 字元的 .ps1 一律存成 UTF-8 with BOM、CRLF
  - 交付腳本前用 5.1 的 parser 驗證：
    `powershell.exe -NoProfile -Command "[System.Management.Automation.Language.Parser]::ParseFile(<path>,[ref]$null,[ref]$e); $e.Count"`
  - 不能假設使用者的提權視窗是 PowerShell 7（`pwsh`）

## 一律用繁體中文回覆（2026-11-05）
- 現象：commit 完成後的回報誤用英文，使用者要求改回中文
- 規則：所有對使用者的說明都用繁體中文；程式碼識別字、指令、技術名詞保留原文
- 特別注意：簡短的確認或回報訊息也一樣，不因內容短就切換語言

## 第三方套件的 log 可能洩漏打字內容（2026-11-05）
- 現象：服務程序的 log 記下每一次按鍵（chewing 在 info 等級記錄 `process #Key(...)`）
- 影響：打字內容被寫進硬碟；已安裝版本累積了數百行
- 規則：
  - logger 只收 `zct*` target 的 info / debug；第三方套件一律 warn 以上
  - 我們自己的 log 不可包含組字、送出文字或按鍵內容
  - 新增依賴後，實際跑一次並檢查 log 內容
