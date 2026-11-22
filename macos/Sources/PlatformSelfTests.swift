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
    let hint = CandidateDisplay(items: ["ㄋㄧ"], labels: [""], highlight: nil, page: 0, totalPages: 1, displayOnly: true)
    _ = view.update(hint, theme: theme)
    try check(view.selection == nil && view.hits.isEmpty, "注音提示不顯示選取底色")
    print("macOS composition caret, target and candidate selection layout passed")
}
