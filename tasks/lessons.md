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

## 「ˇ 要按兩次」是要單獨輸出聲調符號，不是打字卡住（2026-11-17）
- 現象：使用者說「ㄋㄚˇ些…第一次輸入ˇ會卡住、要按兩次ˇ才會輸出音節」，我當成組字/IPC 逾時去查日誌、量延遲、懷疑死鍵
- 實際需求：要輸出「ˇ」這個符號本身（像 ㄟ + 空白輸出「ㄟ」），第一次按只進注音區
- 規則：
  - 使用者描述的症狀跟核心測試結果對不上時，先用一句話複述「你按了什麼、預期看到什麼、實際看到什麼」確認，再開始查
  - 單一按鍵的行為問題先在測試裡把各種按法（單按、連按、接空白）列成表，對照使用者描述找出哪一種符合

## 加設定欄位後 macOS self-test 失敗
- 現象：catalog 加了 candidates.language_model，Windows CI 全過，macOS 的 `--self-test` 報「共用設定欄位完整性」
- 原因：macos/Sources/main.swift 寫死欄位數（43），docs/macos.md 也寫了數字；Windows 本機跑不到 Swift
- 規則：改 zct-preferences catalog 時，一併 grep `fields.count` 與文件中的欄位數一起更新

## 回覆語言
- 使用者糾正：「使用中文和我討論」（我整則回覆用了英文）
- 規則：所有回覆、說明、表格一律繁體中文；寫完回覆前檢查語言
- 再犯（2026-11-19）：長任務結束後的總結報告寫成英文。長報告、表格標題、截圖說明表格的欄位名都要中文（檔名、程式識別字除外）

## 英文單字「打到一半」的假設
- 我說「還在打的單字 Backspace 一次刪一個字母」，但中文模式下按第二個空白前畫面是注音，根本沒有看得到的半個單字
- 規則：描述輸入行為前，先確認使用者實際的打字流程（中文模式自動英文 vs Shift 英文模式），用測試印出每一步畫面再下結論

## 協定版本改了，已開著的 App 只能打英文
- 現象：裝了協定 19 的開發版後，改版前開著的 App 只能打英文
- 原因：舊 DLL 叫起舊服務程序；舊服務程序啟動時發現有新版（協定不同）又「沒有連線」，立刻結束，舊 DLL 永遠連不上
- 規則：改 PROTOCOL_VERSION 前，先在開著的 App 裡實測升級（舊 App 繼續打中文）；新舊並存的流程要考慮「剛啟動、還沒連線」這個時間點

## 截圖不能截到其他視窗（2026-11-19）
- 用螢幕座標截圖（CopyFromScreen）前，先確認目標視窗真的在那個範圍最上層：設成置頂，抽查範圍內多個點的 `WindowFromPoint` 都屬於它，不符就不截
- 程序的 `MainWindowHandle` 不一定是看得到的那個視窗（Slint 有另一個頂層視窗），要列舉程序的可見視窗
- Windows 11 的外框 1px 半透明、圓角外露出背景：視窗放大 2px 只截內部，並關掉圓角
- Slint（GPU 繪製）用 PrintWindow 只會得到黑畫面

## commit 前跑完 CI 的同一組檢查（2026-11-20）
- 現象：改完程式跑過 `cargo fmt`，之後又補了測試沒再格式化就 commit，CI 的格式檢查失敗；後面幾次推送又把舊的 CI 取消，問題晚了好幾個 commit 才發現
- 規則：每次 commit 前依序跑 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`（開發版 features 也跑一次）、`cargo test --workspace --locked`
- 刪掉或改名 Rust／Swift 的函式時，`grep` 整個 `macos/Sources`（含 self-test、UIPreview）找呼叫的地方；Swift 只有 CI 會編譯

## Backspace 規則記反了（2026-11-21）
- 使用者原話：「在輸入英文或是輸入注音的過程中，按下 bs 移除整個 wording，但是一旦輸入完成之後 bs 只會移除一個 char」，以空白判斷完成
- 我卻記成相反（打字中一次退一鍵、完成後整個刪），之後的實作、測試、README、todo 都跟著錯；使用者以為功能「又不見了」
- 規則：把使用者的規格寫進 todo 時，直接引用原話並附上「打字中／完成後」各一個例子；討論中我改寫的版本要給使用者確認，不要用我的摘要當規格
