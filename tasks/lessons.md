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

## 不要用 Python 文字模式改寫腳本檔（2026-11-05）
- 現象：`fetch-dict.sh` 第二次執行時產生空的 `english.txt`，中英混打測試全部失敗
- 原因：
  - 第一次用 Python 寫入時，`'\r'` 變成真的 CR 字元放進引號裡（剛好仍能運作）
  - 第二次 Python 以文字模式讀回，把單獨的 CR 轉成換行，`tr` 指令被切成兩行
  - Windows 上 Python 文字模式寫檔也會把 LF 轉成 CRLF
- 規則：
  - 含跳脫字元（`\r`、`\`）的內容用 Edit 工具修改，不用 Python 字串替換
  - 必須用 Python 時：`open(p, newline='')` 讀寫，並在替換後 `grep` 檢查結果
  - 改完會被重新執行的腳本，立刻執行一次並檢查產出（例如行數）

## 模擬鍵盤輸入前必須確認目標視窗在前景（2026-11-06）
- 現象：tsf_probe 用 SendInput 測試，但測試視窗沒有取得前景，按鍵可能送進使用者正在用的視窗
- 規則：
  - SendInput / keybd_event 這類全域輸入，每次送出前都檢查 `GetForegroundWindow()` 是測試視窗，否則立刻停止
  - 會搶前景或送全域輸入的測試，先徵得使用者同意再執行
  - 能用不影響使用者的方式（直接呼叫函式、IPC 模擬）驗證時優先使用

## Bash 工具的 heredoc 會吃掉反斜線（2026-11-07）
- 現象：在 Bash heredoc 裡跑 Python 修改檔案，`\\` 變成 `\`，`\r` 被寫成真正的歸位字元、字串比對失敗；腳本中途失敗但後面的 git commit 照樣執行
- 規則：
  - 內容含反斜線（Windows 路徑、跳脫字元）的修改，用 Write 把腳本寫到 scratchpad 再執行，或直接用 Edit
  - 修改腳本與 git commit 不要用 `&&` 以外的方式串接；失敗時先檢查 `git diff` 再 commit

## 自動測試會改到使用者真正的設定（2026-11-08）
- 現象：用 tsf_probe 測組字提示列快捷鍵時，服務程序把結果寫進使用者的 `%APPDATA%\zct\apps.toml`（新增一條 tsf_probe 規則）
- 規則：
  - 會寫入設定或學習資料的測試，在獨立的資料資料夾跑（`ZCT_HOME`），或測試前後比對並還原
  - 測試前先記下會被改到的檔案；需要刪除使用者檔案時先告知使用者

## 協定版本改變後，已開著的 App 會連不上（2026-11-08）
- 現象：重新安裝後，VS Code、Discord、檔案總管還載入舊 DLL（協定 10），新服務程序是 11，log 出現「協定版本不符」，這些 App 暫時沒有輸入法
- 規則：
  - 改了 IPC 格式就要提醒使用者重開哪些 App（看服務程序 log 的「協定版本不符」）
  - 發佈正式版前完成協定向下相容（M10），自動更新後舊 App 才能繼續用

## PowerShell 的 $null 傳給 C# string 參數會變成空字串（2026-11-09）
- 現象：install-dev.ps1 用 `MoveFileEx(path, $null, DELAY_UNTIL_REBOOT)` 排程刪除舊版，結果什麼都沒排；回傳值被 `[void]` 吃掉，沒有任何錯誤
- 原因：PowerShell 呼叫 .NET / P/Invoke 時把 `$null` 轉成 `""`，MoveFileEx 收到空字串而失敗
- 規則：
  - 要傳 null 給 string 參數時用 `[NullString]::Value`
  - P/Invoke 的回傳值不要用 `[void]` 丟掉；失敗時印出 `GetLastWin32Error()`
  - 腳本的實際效果要檢查（例如 PendingFileRenameOperations），不能只看腳本有沒有跑完

## 改執行檔名稱會改到模組路徑（2026-11-12）
- 現象：服務程序改名為 zoo-type-service 後，log 裡沒有任何自己的 info 訊息
- 原因：logger 用 `target().starts_with("zct")` 分辨自己與第三方；bin 改名後模組路徑變成 `zoo_type_service::…`，全被當成第三方過濾
- 規則：
  - 改 crate / bin / lib 名稱前，搜尋所有依名稱判斷的地方（log 過濾、`module_path!`、`env!("CARGO_PKG_NAME")`、腳本中的檔名）
  - 改名後實際啟動一次，檢查 log 有正常的啟動訊息

## install-dev.ps1 不會建置：資料夾標著新 commit，裝的卻是舊程式（2026-11-13）
- 現象：連續兩次安裝（9b6332f、cbb5d3d）前沒跑 `cargo build --release`，裝進去的都是早上 0fea6c7 的程式；使用者打開設定看到舊的 placeholder、加縮寫報「注音格式錯誤」
- 規則：
  - 安裝前一定先 `cargo build --release --workspace`（install-dev.ps1 現在會檢查，建置比原始碼舊就拒絕）
  - 回報「已裝好」之前，確認安裝資料夾的程式真的包含這次的修改（例如搜尋新加的字串）

## UI 文字要照最終規格寫，不是在舊文字上補註解（2026-11-13）
- 現象：詞庫讀音欄 placeholder 原本是「注音（自動填入）」，加縮寫時改成「注音或英文縮寫」，Help 也在尾巴再補一段縮寫說明
- 使用者要求：placeholder 只寫欄位是什麼（「注音」），不寫「自動填入」這類附加行為；功能增加時重新設計整段文字，而不是一直附加說明
- 規則：
  - 欄位標籤、placeholder 只說「這是什麼」；行為說明盡量讓版面本身表達（CLAUDE.md：Let the Layout Explain）
  - 新功能改到既有文字時，以「如果一開始就有這個功能會怎麼寫」重寫整段

## 吃掉放開事件會讓 App 以為按鍵還按著（2026-11-13）
- 現象：單按 Shift 切中英後，設定程式按 Tab 焦點往回跳到「儲存設定」（Slint 當成 Shift + Tab）
- 原因：OnTestKeyUp 回傳 TRUE 時 TSF 就吃掉放開事件，OnKeyUp 回傳什麼都一樣
- 規則：
  - 放開事件一律交給 App；需要更新文件時從 OnTestKeyUp 用非同步 edit session（TF_ES_ASYNCDONTCARE）
  - 只在真實視窗出現的問題，先徵得同意再用模擬按鍵實測（UI Automation 讀焦點與欄位內容），不要只靠推測
