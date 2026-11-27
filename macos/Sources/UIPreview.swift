import AppKit

// Offscreen rendering uses the same native views as the installed input method.
func uiSelfTest(dictionary: String, output: URL) throws {
    let app = NSApplication.shared
    app.setActivationPolicy(.prohibited)
    let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: temporary) }
    setenv("ZCT_SELF_TEST", "1", 1)
    setenv("ZCT_HOME", temporary.path, 1)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
    try platformSelfTests()
    let runtime = try EngineRuntime(dictionary: dictionary)
    let settings = try SettingsWindowController(runtime: runtime)
    func snapshot(_ view: NSView, _ filename: String) throws {
        view.layoutSubtreeIfNeeded()
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else {
            throw EngineError(description: "無法繪製 UI 預覽")
        }
        view.cacheDisplay(in: view.bounds, to: rep)
        try rep.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(filename))
    }
    guard let window = settings.window, let content = window.contentView else { throw EngineError(description: "設定視窗不存在") }
    window.appearance = NSAppearance(named: .aqua)
    window.displayIfNeeded()
    try snapshot(content, "settings-general.png")
    try settingsAcceptanceTest(runtime: runtime, capture: snapshot)
    try typingPopupAcceptanceTest(runtime: runtime, capture: snapshot)
    settings.previewPage(2)
    window.displayIfNeeded()
    try snapshot(content, "settings-appearance.png")
    let panel = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 240, height: 200), styleMask: .borderless, backing: .buffered, defer: false)
    let candidate = CandidateView()
    panel.contentView = candidate
    var theme = CandidateTheme()
    let items = ["天氣", "天汽", "添氣", "填氣", "田契"]
    for (filename, horizontal, columns) in [("candidate-vertical.png", false, [] as [[String]]),
                                           ("candidate-horizontal.png", true, []),
                                           ("candidate-columns.png", false, [items, ["天器", "添器", "田氣"]])] {
        theme.horizontal = horizontal
        let size = candidate.update(CandidateDisplay(items: items, labels: ["1★", "2", "3", "4", "5"], columns: columns,
                                    highlight: 0, page: 0, totalPages: 3), theme: theme)
        panel.setContentSize(size)
        try snapshot(candidate, filename)
    }
    guard let icon = NSImage(contentsOf: Bundle.main.resourceURL!.appendingPathComponent("menu.tiff")),
          icon.size == NSSize(width: 16, height: 16) else {
        throw EngineError(description: "選單圖示必須是 16 point，避免輸入法選單列被撐高")
    }
    print("macOS UI rendering / menu icon self-test passed: \(output.path)")
}

// Click the real AppKit controls against an isolated Rust-backed settings store.
private func settingsAcceptanceTest(runtime: EngineRuntime, capture: (NSView, String) throws -> Void) throws {
    func check(_ condition: Bool, _ message: String) throws {
        if !condition { throw EngineError(description: "設定驗收：" + message) }
    }
    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    let initial = try PreferencesModel(client: runtime)
    // 組字提示列快捷鍵寫回 apps.toml 的行為在共用層（zct-host 測試）；這裡只驗設定畫面保留跨平台欄位
    initial.apps = [["name": "Editor", "macos": "test.Editor", "windows": "editor.exe", "detect": false, "composition_bar": true],
                    ["name": "test.Terminal", "macos": "test.Terminal", "composition_bar": false]]
    try initial.save()
    try initial.reload()
    try check(initial.apps.count == 2 && initial.apps[0]["windows"] as? String == "editor.exe" &&
              initial.apps[0]["detect"] as? Bool == false && initial.apps[0]["composition_bar"] as? Bool == true &&
              initial.apps[1]["composition_bar"] as? Bool == false, "App 規則保留跨平台欄位")
    let original: [String: Any] = [
        "candidates.habit": "custom", "candidates.select": "before", "candidates.after_select": "backward",
        "input.output": "simplified", "input.on_focus": "reset", "toggle.shift_tap_max_ms": 321,
        "punctuation.primary": "both", "punctuation.secondary": "layer",
        "candidates.sort_by_frequency": false, "detect.fullscreen": false,
        "punctuation.full": ["shift+,": "，"]
    ]
    for (path, value) in original { initial.set(path, value) }
    try initial.save()
    var settings = try SettingsWindowController(runtime: runtime)
    func root() throws -> NSView {
        guard let view = settings.window?.contentView else { throw EngineError(description: "設定視窗不存在") }
        return view
    }
    func field(_ path: String) throws -> NSControl? {
        guard let descriptor = initial.fields.first(where: { $0.path == path }) else { throw EngineError(description: "欄位不存在：" + path) }
        let views = descendants(try root())
        if descriptor.kind == "bool" { return views.compactMap { $0 as? NSButton }.first { $0.title == descriptor.label } }
        guard let label = views.compactMap({ $0 as? NSTextField }).first(where: { !$0.isEditable && $0.stringValue == descriptor.label }),
              let row = label.superview as? NSStackView else { return nil }
        return row.arrangedSubviews.compactMap { $0 as? NSControl }.first { $0 !== label }
    }
    func click(_ title: String) throws {
        guard let control = descendants(try root()).compactMap({ $0 as? NSButton }).first(where: { $0.title == title }) else {
            throw EngineError(description: "找不到按鈕：" + title)
        }
        control.performClick(nil)
    }
    func choose(_ path: String, _ value: String) throws {
        guard let popup = try field(path) as? NSPopUpButton,
              let descriptor = initial.fields.first(where: { $0.path == path }),
              let index = descriptor.choices.firstIndex(where: { $0["value"] as? String == value }) else {
            throw EngineError(description: "找不到選項：" + path + " / " + value)
        }
        popup.selectItem(at: index)
        _ = popup.sendAction(popup.action, to: popup.target)
    }
    func selected(_ path: String) throws -> String? { (try field(path) as? NSPopUpButton)?.titleOfSelectedItem }
    let advanced = ["input.output", "input.on_focus", "toggle.shift_tap_max_ms", "punctuation.primary", "punctuation.secondary", "candidates.sort_by_frequency", "detect.fullscreen"]
    for path in advanced { try check(try field(path) == nil, path + " 預設應收合") }
    try click("▸ 進階")
    for path in advanced { try check(try field(path) != nil, path + " 展開後應存在") }
    settings.window?.setContentSize(NSSize(width: 860, height: 1100))
    try capture(try root(), "settings-advanced.png")
    try click("▾ 進階")
    for preset in ["windows", "macos"] {
        try choose("candidates.habit", preset)
        try check(try field("candidates.select") == nil && field("candidates.after_select") == nil, preset + " 應隱藏自訂欄位")
        try capture(try root(), "settings-habit-" + preset + ".png")
    }
    try choose("candidates.habit", "custom")
    try check(try selected("candidates.select") == "游標前的詞" && selected("candidates.after_select") == "移到詞前面", "切回自訂必須保留原值")
    // 同一頁換選項會重畫整頁：捲動位置要保留，不能跳回最上面
    settings.window?.setContentSize(NSSize(width: 860, height: 360))
    try root().layoutSubtreeIfNeeded()
    guard let scroll = descendants(try root()).compactMap({ $0 as? NSScrollView }).first(where: { $0.documentView is NSStackView }),
          let page = scroll.documentView else { throw EngineError(description: "設定頁捲動區不存在") }
    let scrolled = min(120, page.frame.height - scroll.contentView.bounds.height)
    try check(scrolled > 0, "設定頁要長到可以捲動")
    scroll.contentView.scroll(to: NSPoint(x: 0, y: scrolled)); scroll.reflectScrolledClipView(scroll.contentView)
    try choose("candidates.habit", "macos")
    try check(abs(scroll.contentView.bounds.origin.y - scrolled) < 1, "換選項後捲動位置應保留")
    try choose("candidates.habit", "custom")
    settings.window?.setContentSize(NSSize(width: 860, height: 1100))
    // Save while the custom controls and advanced controls are both hidden.
    try choose("candidates.habit", "windows")
    guard let count = try field("candidates.per_page") as? NSTextField else { throw EngineError(description: "每頁候選字數欄位不存在") }
    count.stringValue = "5"
    try click("儲存設定")
    let saved = try PreferencesModel(client: runtime)
    try check((saved.value("candidates.per_page") as? NSNumber)?.intValue == 5, "按儲存必須寫入檔案")
    for (path, value) in original where path != "candidates.habit" {
        try check((saved.value(path) as? NSObject)?.isEqual(value) == true, "隱藏欄位不應被覆寫：" + path)
    }
    settings.close()
    settings = try SettingsWindowController(runtime: runtime)
    try check(try selected("candidates.habit") == "Windows", "重開後操作習慣應保留")
    try choose("candidates.habit", "custom")
    try check(try selected("candidates.select") == "游標前的詞" && selected("candidates.after_select") == "移到詞前面", "重開後自訂值必須保留")
    try click("▸ 進階")
    try check(try selected("input.output") == "簡體" && selected("input.on_focus") == "重設為預設模式", "進階設定必須保留")
    try check((try field("toggle.shift_tap_max_ms") as? NSTextField)?.stringValue == "321", "按住上限必須保留")
    try capture(try root(), "settings-reopened.png")
    settings.close()

    // Open and save a legacy file through the native UI and actual preferences API.
    try "[mixed]\nauto_english = false\n[detect]\nlegacy_fullscreen = false\n[theme]\nfont_size = 19.5\n".write(to: runtime.configURL, atomically: true, encoding: .utf8)
    settings = try SettingsWindowController(runtime: runtime)
    try check(try selected("mixed.english") == "關", "舊 auto_english 必須可讀取")
    try click("儲存設定")
    let migrated = try String(contentsOf: runtime.configURL, encoding: .utf8)
    for legacy in ["auto_english", "legacy_fullscreen", "exclusive_fullscreen"] { try check(!migrated.contains(legacy), "儲存後應移除舊欄位 " + legacy) }
    let model = try PreferencesModel(client: runtime)
    try check(model.value("detect.fullscreen") as? Bool == false, "舊全螢幕開關值必須保留")
    try check((model.value("theme.font_size") as? NSNumber)?.doubleValue == 19.5, "轉換不能改掉其他設定")
    settings.close()
    print("macOS settings acceptance passed: advanced / presets / save-reopen / legacy migration")
}

private func typingPopupAcceptanceTest(runtime: EngineRuntime, capture: (NSView, String) throws -> Void) throws {
    func check(_ condition: Bool, _ message: String) throws {
        if !condition { throw EngineError(description: "組字小視窗驗收：" + message) }
    }
    let panel = CandidatePanel()
    defer { panel.orderOut(nil) }
    var time: UInt64 = 1000
    for display in ["keys", "keys_and_chars"] {
        let preferences = try PreferencesModel(client: runtime)
        preferences.set("mixed.typing", display)
        try preferences.save()
        let session = try EngineSession(runtime: runtime, app: "com.apple.TextEdit")
        func press(_ code: UInt16) throws -> EngineResult {
            time += 30
            let result = try session.key(code: code, modifiers: 0, time: time)
            time += 30
            _ = try session.key(code: code, modifiers: 0, released: true, time: time)
            return result
        }
        _ = try press(1) // s
        let partial = try press(32) // u
        try check(partial.composition.text == "su" && partial.candidatePage.displayOnly, "組字區應是 su")
        try check(partial.candidatePage.items.first == "ㄋㄧ", "小視窗應有 ㄋㄧ")
        if display == "keys" { try check(partial.candidatePage.items == ["ㄋㄧ"], "只顯示注音") }
        else { for word in ["你", "妳", "尼"] { try check(partial.candidatePage.items.contains(word), "應列出常用字 " + word) } }
        let theme = CandidateTheme(config: preferences.config)
        let caret = NSRect(x: 300, y: 500, width: 1, height: 18)
        panel.show(result: partial, plan: session.popupPlan(partial, compositionBar: false, caret: caret, fresh: true), theme: theme)
        try check(panel.candidates.isVisible && !panel.bar.isVisible, "打字時應顯示原生小視窗")
        try capture(panel.candidates.view, "typing-popup-" + display + ".png")
        let alone = panel.candidates.frame
        // 終端機的組字提示列：注音在自己的視窗，大小與位置和沒有提示列時一樣（不會一行兩行地跳）；提示列放在游標上方
        panel.show(result: partial, plan: session.popupPlan(partial, compositionBar: true, caret: caret, fresh: true), theme: theme)
        try check(panel.bar.isVisible && panel.candidates.isVisible, "提示列與注音各自一個視窗")
        try check(panel.candidates.frame == alone, "有提示列時注音視窗不變")
        try check(panel.bar.frame.minY >= caret.maxY && panel.candidates.frame.maxY <= caret.minY, "提示列在游標上方、注音在下方")
        try capture(panel.bar.view, "typing-popup-" + display + "-bar.png")
        let toned = try press(20) // 3
        try check(toned.composition.text == "你" && !toned.candidatePage.displayOnly && toned.candidatePage.items.isEmpty, "打完聲調應結束提示")
        panel.show(result: toned, plan: session.popupPlan(toned, compositionBar: false, caret: caret, fresh: true), theme: theme)
        try check(!panel.candidates.isVisible && !panel.bar.isVisible, "打完聲調小視窗應立即消失")
        // 切換中英等提示：自己一個視窗，放在游標下方，不影響其他視窗
        time += 100
        _ = try session.key(code: 56, modifiers: 1, time: time) // 單按左 Shift
        let toggle = try session.key(code: 56, modifiers: 0, released: true, time: time + 50)
        time += 100
        try check(!toggle.notice.isEmpty, "切換中英應有提示")
        panel.show(result: toggle, plan: session.popupPlan(toggle, compositionBar: true, caret: caret, fresh: true), theme: theme)
        try check(panel.hint.isVisible && !panel.candidates.isVisible && panel.hint.frame.maxY <= caret.minY, "提示在游標下方自己的視窗")
        try capture(panel.hint.view, "typing-popup-" + display + "-hint.png")
        panel.orderOut(nil)
        _ = try session.key(code: 56, modifiers: 1, time: time)
        _ = try session.key(code: 56, modifiers: 0, released: true, time: time + 50) // 切回中文
    }
    try wezTermInlineAcceptance(runtime: runtime) { result in
        let caret = NSRect(x: 200, y: 300, width: 1, height: 18)
        panel.show(result: result, plan: EngineSession.popupPlan(nil, result, compositionBar: true, caret: caret, fresh: true), theme: CandidateTheme())
        try check(panel.bar.isVisible, "WezTerm 組字提示列")
    }
    print("macOS typing popup acceptance passed")
}

func wezTermInlineAcceptance(runtime: EngineRuntime, inspect: (EngineResult) throws -> Void = { _ in }) throws {
    func check(_ value: Bool, _ message: String) throws {
        if !value { throw EngineError(description: message) }
    }
    var time: UInt64 = 1000
    let terminalID = "com.github.wez.wezterm"
    let terminalBar = InputController.isTerminal(terminalID)
    try check(terminalBar, "WezTerm 預設顯示組字列")
    let preferences = try PreferencesModel(client: runtime)
    for priority in ["chinese_first", "english_first"] {
        preferences.set("mixed.english", priority)
        try preferences.save()
        let session = try EngineSession(runtime: runtime, app: terminalID)
        var previous: EngineResult?
        var inline = ""
        var lastDelivered = ""
        func stroke(_ code: UInt16) throws -> EngineResult {
            var down: EngineResult?
            for released in [false, true] {
                time += 30
                let result = try session.key(code: code, modifiers: 0, released: released, time: time)
                if !released {
                    down = result
                    inline = "" // WezTerm clears its local buffer before interpretKeyEvents.
                }
                if InputController.shouldSendMarkedText(result, appID: terminalID, handledKeyDown: !released && result.eaten,
                        marked: previous != nil, previousText: previous?.composition.text ?? "", cursor: previous?.composition.cursor ?? -1,
                        target: previous?.composition.target ?? NSRange(), replacement: NSRange(location: NSNotFound, length: 0)) {
                    InputController.writeMarkedText(result, appID: terminalID,
                        repeatText: !released && result.eaten && previous?.composition.text == result.composition.text,
                        replacement: NSRange(location: NSNotFound, length: 0)) { payload, selection, _ in
                        guard let text = payload as? String, selection.location == text.utf16.count else { return }
                        if text != lastDelivered { inline = text; lastDelivered = text }
                    }
                }
                try check(!inline.isEmpty && inline == result.composition.text, "WezTerm 行內組字遺失，鍵碼 " + String(code))
                previous = result
                try inspect(result)
            }
            return down!
        }
        _ = try stroke(37)
        _ = try stroke(37)
        _ = try session.flush()
        previous = nil
        lastDelivered = ""
        if priority == "chinese_first" {
            for code: UInt16 in [3, 32, 40] { _ = try stroke(code) } // ㄑㄧㄜ
            let invalid = try stroke(49)
            try check(invalid.bell, "無效一聲應保留行內音節")
            time += 30
            let removed = try session.key(code: 51, modifiers: 0, time: time)
            try check(removed.composition.text.isEmpty && removed.commit.isEmpty, "一次退掉無效音節")
            previous = nil
            lastDelivered = ""
        }
        for code: UInt16 in [4, 14, 37, 37, 31] { _ = try stroke(code) } // hello
        _ = try session.flush()
        previous = nil
        for code: UInt16 in [13, 32, 29, 49, 3, 32, 21] { _ = try stroke(code) } // 天氣
        _ = try stroke(115) // Home: select a character before the end.
        let menu = try stroke(125)
        try check(!menu.candidatePage.items.isEmpty && !menu.candidatePage.displayOnly, "WezTerm 可開始選字")
        let moved = try stroke(125)
        try check(!moved.candidatePage.items.isEmpty && moved.candidatePage.highlight != menu.candidatePage.highlight, "WezTerm 移動選字仍保有候選")
        _ = try stroke(36) // Enter accepts the highlighted candidate inside the composition.
    }
    print("WezTerm inline composition acceptance passed")
}
