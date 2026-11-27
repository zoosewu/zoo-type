import AppKit

func platformSelfTests() throws {
    func check(_ condition: Bool, _ message: String) throws {
        if !condition { throw EngineError(description: "平台驗收：" + message) }
    }
    let service = "ZooType.keychain-test." + UUID().uuidString
    defer { try? Credentials.write("", service: service) }
    try check(try Credentials.read(service: service) == nil, "Keychain 新項目")
    try Credentials.write("fixture-only-123", service: service)
    try check(try Credentials.read(service: service) == "fixture-only-123", "Keychain 寫入與讀回")
    try Credentials.write("fixture-updated", service: service)
    try check(try Credentials.read(service: service) == "fixture-updated", "Keychain 修改")
    try Credentials.write("", service: service)
    try check(try Credentials.read(service: service) == nil, "Keychain 清除")
    // 不在輸入狀態的判斷規則在共用層（zct_core::detect 與 zct-ffi 的測試）

    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 300), styleMask: .borderless, backing: .buffered, defer: false)
    let view = CandidateView(); window.contentView = view
    var picked: (Int, Int)?
    view.onChoose = { picked = ($0, $1) }
    let theme = CandidateTheme()
    for columns in [false, true] {
        let size = view.update(CandidateDisplay(items: ["你", "妳", "尼"], labels: ["1", "2", "3"], columns: columns ? [["你", "妳"], ["尼", "泥"]] : [], firstColumn: 2, highlight: 0, page: 2, totalPages: 4), theme: theme)
        window.setContentSize(size)
        guard let hit = view.hits.last else { throw EngineError(description: "滑鼠點擊範圍不存在") }
        let point = view.convert(NSPoint(x: hit.rect.midX, y: hit.rect.midY), to: nil)
        let event = NSEvent.mouseEvent(with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1)!
        view.mouseDown(with: event)
        try check(picked?.0 == hit.page && picked?.1 == hit.row, "滑鼠命中正確候選頁與列")
    }
    _ = view.update(CandidateDisplay(items: [], labels: [], highlight: 0, page: 0, totalPages: 0), notice: "ㄋㄧ", theme: theme)
    try check(view.hits.isEmpty, "注音提示不能被滑鼠選取")
    // 更新不降版、只接受官方來源：在共用層（zct-host update 測試）

    let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: temporary) }
    let old = temporary.appendingPathComponent("old.app"), new = temporary.appendingPathComponent("new.app")
    try FileManager.default.createDirectory(at: old, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: new, withIntermediateDirectories: true)
    try "old".write(to: old.appendingPathComponent("fixture"), atomically: true, encoding: .utf8)
    try "new".write(to: new.appendingPathComponent("fixture"), atomically: true, encoding: .utf8)
    let backup = try UpdateFiles.replaceApp(staged: new, installed: old)
    try check(try String(contentsOf: old.appendingPathComponent("fixture"), encoding: .utf8) == "new", "更新原子替換")
    try check(try String(contentsOf: backup.appendingPathComponent("fixture"), encoding: .utf8) == "old", "更新保留舊版")
    try check(try FileManager.default.contentsOfDirectory(atPath: temporary.path).sorted() == ["new.app", "old.app"], "更新不在安裝資料夾留下舊版")
    try FileManager.default.removeItem(at: backup.deletingLastPathComponent())
    try FileManager.default.createDirectory(at: temporary.appendingPathComponent("old.app.previous-1"), withIntermediateDirectories: true)
    UpdateFiles.removeLeftovers(beside: old)
    try check(try FileManager.default.contentsOfDirectory(atPath: temporary.path).sorted() == ["new.app", "old.app"], "清除舊版留下的備份")
    // 修飾鍵：T3（Chromium）每個 Shift 事件送兩次（記錄：按下 mods=1 兩次、放開兩次），重複的不能當成放開
    let shift = NSEvent.ModifierFlags.shift.rawValue, left: UInt = 0x2, right: UInt = 0x4
    try check(InputController.modifierPressed(56, flags: shift | left, down: []) == true, "按下左 Shift")
    try check(InputController.modifierPressed(56, flags: shift | left, down: [56]) == true, "重複的按下仍是按著（呼叫端略過）")
    try check(InputController.modifierPressed(56, flags: 0, down: [56]) == false, "放開左 Shift")
    try check(InputController.modifierPressed(56, flags: shift | right, down: [56, 60]) == false, "兩邊都按著時放開左邊")
    try check(InputController.modifierPressed(56, flags: shift, down: [56]) == true, "合成事件沒有左右位元：重複的按下")
    try check(InputController.modifierPressed(56, flags: shift, down: [56, 60]) == false, "合成事件：兩邊都按著時放開一邊")
    try check(InputController.modifierPressed(0, flags: shift, down: []) == nil, "不是修飾鍵")
    let validation = Result { try UpdateFiles.validateApp(old, version: "0.1.5", bundleID: Bundle.main.bundleIdentifier!) }
    if case .success = validation { throw EngineError(description: "不合法的 App 未拒絕") }
    print("macOS platform acceptance passed: Keychain / detection / mouse / update files")
}

func compositionCaretSelfTest() throws {
    func check(_ condition: Bool, _ message: String) throws {
        if !condition { throw EngineError(description: message) }
    }
    let view = CandidateView()
    let theme = CandidateTheme()
    let composed = "天😀氣"
    var previousCaret: CGFloat = -1
    for offset in [0, 1, 3, 4] {
        let range = offset == 4 ? NSRange() : NSRange(location: offset, length: offset == 1 ? 2 : 1)
        _ = view.update(CandidateDisplay(items: ["氣"], labels: ["1"], highlight: 0, page: 0, totalPages: 1), composition: CompositionDisplay(text: composed, cursor: offset, target: range), theme: theme)
        guard let caret = view.compositionCaret else { throw EngineError(description: "缺少組字游標") }
        try check(caret.minX > previousCaret, "組字游標隨 UTF-16 位置移動")
        previousCaret = caret.minX
        if range.length > 0 {
            let target = view.compositionTarget ?? .zero
            try check(target.width > 0 && target.minX <= caret.midX && caret.midX < target.maxX, "選字目標背景")
        } else { try check(view.compositionTarget == nil, "句尾沒有目標字") }
    }
    for row in [0, 1] {
        let page = CandidateDisplay(items: ["氣", "汽"], labels: ["1", "2★"], highlight: row, page: 0, totalPages: 1)
        _ = view.update(page, theme: theme)
        try check(view.selection == view.hits[row].rect, "選取底色應跟隨候選列")
    }
    // 量 App 的游標：組字區 3 個字在文件第 17 個字開始，每個字寬 9、從 x=564 起；游標在最後面（第 3 格）
    let length = 3, marked = NSRange(location: 17, length: length)
    let x = { (i: Int) in 564 + 9 * CGFloat(i) }
    let none = NSRange(location: NSNotFound, length: 0)
    func caretX(cursor: Int = length, marked: NSRange = marked, _ attributes: (Int) -> NSRect) -> CGFloat? {
        InputController.measureCaret(cursor: cursor, marked: marked, attributes: attributes)?.minX
    }
    // 一般 App：從組字區開頭算，回報字寬
    let cocoa = { (i: Int) in i < length ? NSRect(x: x(i), y: 207, width: 9, height: 16) : .zero }
    try check(caretX({ i in i <= length ? NSRect(x: x(i), y: 207, width: 1, height: 16) : .zero }) == x(3), "App 回報組字區最後面")
    try check(caretX(cocoa) == x(3), "最後面量不到：用最後一個字的右緣")
    try check(caretX(cursor: 1, cocoa) == x(1), "游標在中間")
    // Chromium（記錄裡的真實行為）：位置是整份文件的，只回答組字區裡的字，寬度只有 1
    let chromium = { (i: Int) in marked.contains(i) ? NSRect(x: x(i - marked.location), y: 207, width: 1, height: 16) : .zero }
    try check(caretX(chromium) == x(3), "Chromium：改用文件位置，用前兩個字的間距推算最後一個字的右緣")
    try check(caretX(cursor: 2, chromium) == x(2), "Chromium：游標在中間")
    try check(caretX(cursor: 1, marked: NSRange(location: 17, length: 1), { $0 == 17 ? NSRect(x: x(0), y: 207, width: 1, height: 16) : .zero }) == x(0) + 1,
              "Chromium 只有一個字：推算不了字寬就放在字的右邊")
    // 組字區在文件開頭：兩種算法一樣（記錄：marked=0+16 時 attributes(8)、(9) 都量得到）
    try check(caretX(cursor: 2, marked: NSRange(location: 0, length: length), cocoa) == x(2), "組字區在文件開頭")
    // 垃圾值（IMK 轉送 firstRect 的結果）一律不採用
    try check(caretX(marked: none, { _ in NSRect(x: 1.6e-314, y: 244497, width: 1.6e-314, height: -1) }) == nil, "不採用垃圾值（右上角）")
    try check(caretX(marked: none, { _ in NSRect(x: 1.6e-314, y: 0, width: 1.6e-314, height: 1.6e-314) }) == nil, "不採用垃圾值（左下角）")
    try check(caretX(marked: none, { _ in .zero }) == nil, "都量不到交給呼叫端沿用")
    let hint = CandidateDisplay(items: ["ㄋㄧ"], labels: [""], highlight: nil, page: 0, totalPages: 1, displayOnly: true)
    _ = view.update(hint, theme: theme)
    try check(view.selection == nil && view.hits.isEmpty, "注音提示不顯示選取底色")
    print("macOS composition caret, target and candidate selection layout passed")
}
