import AppKit
import InputMethodKit
import os

@objc(ZooTypeInputController)
final class InputController: IMKInputController {
    private var engine: EngineSession?
    private var panel: CandidatePanel?
    private var marked = false
    private var currentPreedit = ""
    private var markedCursor = -1
    private var markedTarget = NSRange(location: 0, length: 0)
    private var modifiersDown: Set<UInt16> = []
    private var appID = ""
    private var mode: UInt32 = 0
    private var theme = CandidateTheme()
    private var showComposition = false
    private var aiTimer: Timer?
    private var settingsTimer: Timer?
    private var game = false
    private var simplified = false
    private var replacement = NSRange(location: NSNotFound, length: 0)
    /// 最近一次 App 回報的游標：剛送出組字文字時有些 App 還量不到，先沿用這個，不跳到滑鼠旁
    private var knownCaret: NSRect?

    override func recognizedEvents(_ sender: Any!) -> Int {
        Int(NSEvent.EventTypeMask.keyDown.rawValue | NSEvent.EventTypeMask.keyUp.rawValue |
            NSEvent.EventTypeMask.flagsChanged.rawValue | NSEvent.EventTypeMask.leftMouseDown.rawValue |
            NSEvent.EventTypeMask.rightMouseDown.rawValue)
    }

    override func activateServer(_ sender: Any!) {
        guard let client = (sender as? IMKTextInput) ?? client() else { return }
        if engine != nil { commitComposition(sender) }
        let clientID = client.bundleIdentifier() ?? ""
        appID = clientID.isEmpty ? (NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "") : clientID
        NotificationCenter.default.removeObserver(self, name: .zctSettingsChanged, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(settingsDidChange(_:)), name: .zctSettingsChanged, object: nil)
        NotificationCenter.default.removeObserver(self, name: .zctWillInstallUpdate, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(prepareUpdate(_:)), name: .zctWillInstallUpdate, object: nil)
        modifiersDown.removeAll()
        replacement = NSRange(location: NSNotFound, length: 0)
        knownCaret = nil
        do {
            guard let runtime = appRuntime else { throw EngineError(description: "輸入引擎未啟動，請查看 Console 的 ZooType 記錄") }
            engine = try EngineSession(runtime: runtime, app: appID)
            let state = try engine!.command(5)
            mode = state.mode
            simplified = state.simplified
            game = state.game
            showComposition = state.compositionBar
            let snapshot = try runtime.preferences(["op": "load"])["snapshot"] as? [String: Any] ?? [:]
            applySettings(snapshot)
            settingsTimer?.invalidate()
            settingsTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.refreshIdleSettings() }
        } catch { NSLog("ZooType activate: %@", String(describing: error)); NSSound.beep() }
    }

    override func deactivateServer(_ sender: Any!) {
        commitComposition(sender)
        aiTimer?.invalidate(); settingsTimer?.invalidate()
        engine = nil
        panel?.orderOut(nil)
        modifiersDown.removeAll()
        appRuntime?.save()
    }

    override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
        guard let event, let client = sender as? IMKTextInput else { return false }
        if event.type == .leftMouseDown || event.type == .rightMouseDown {
            commitComposition(sender)
            return false
        }
        guard let engine else { return false }
        var caret = NSRect.zero
        _ = client.attributes(forCharacterIndex: max(0, markedCursor), lineHeightRectangle: &caret)
        // Terminal grids can omit both a selection and caret during a TUI redraw.
        let hasTextContext = Self.isTerminal(appID) || marked || !engine.isIdle || client.selectedRange().location != NSNotFound
        if caret.height > 0 { knownCaret = caret }
        if Self.debugKeys {
            Self.caretLog.notice("ZooType caret 按鍵前 \(self.appID, privacy: .public) attributes(\(max(0, self.markedCursor)))=\(NSStringFromRect(caret), privacy: .public)")
        }
        let signals = InputDetection.gather(hasTextContext: hasTextContext, hasCaret: caret.height > 0)
        if engine.notInput(signals) {
            if Self.debugKeys {
                Self.caretLog.notice("ZooType key 不是輸入框，不處理 keyCode=\(Self.functionKeys.contains(event.keyCode) ? Int(event.keyCode) : -1) secure=\(signals.secure) text=\(signals.hasTextContext) caret=\(signals.hasCaret) fullscreen=\(signals.fullscreen)")
            }
            aiTimer?.invalidate(); panel?.orderOut(nil); modifiersDown.removeAll()
            return false
        }
        var mods: UInt32 = 0
        if event.modifierFlags.contains(.shift) { mods |= 1 }
        if event.modifierFlags.contains(.control) { mods |= 2 }
        if event.modifierFlags.contains(.option) { mods |= 4 }
        if event.modifierFlags.contains(.command) { mods |= 8 }
        if event.modifierFlags.contains(.capsLock) { mods |= 16 }
        var released = event.type == .keyUp
        if event.type == .flagsChanged {
            // 狀態沒變的是重複送來的事件（Chromium 系 App 每次都送兩次），不能當成放開
            guard let pressed = Self.modifierPressed(event.keyCode, flags: event.modifierFlags.rawValue, down: modifiersDown),
                  pressed != modifiersDown.contains(event.keyCode) else { return false }
            released = !pressed
            if released { modifiersDown.remove(event.keyCode) } else { modifiersDown.insert(event.keyCode) }
        }
        if modifiersDown.contains(61) { mods |= 32 }
        do {
            let result = try engine.key(code: event.keyCode, modifiers: mods, released: released,
                                        time: UInt64(max(0, event.timestamp * 1000)))
            Self.debugKey(event, mods: mods, released: released, result: result)
            render(result, client: client, handledKeyDown: event.type == .keyDown && result.eaten)
            if result.aiPending { pollAI(engine) }
            if result.reconvert { reconvert(client: client) }
            return event.type == .flagsChanged ? false : result.eaten
        } catch {
            NSLog("ZooType key: %@", String(describing: error))
            commitComposition(sender)
            return false
        }
    }

    /// 這個修飾鍵現在是不是按著；nil 表示不是修飾鍵。
    /// 優先看左右鍵各自的位元（NX_DEVICE*KEYMASK）；沒有的話（合成事件）退回看共用旗標與已按下的鍵
    static func modifierPressed(_ keyCode: UInt16, flags: UInt, down: Set<UInt16>) -> Bool? {
        let flag: NSEvent.ModifierFlags, device: UInt, siblingDevice: UInt, sibling: UInt16
        switch keyCode {
        case 56: (flag, device, siblingDevice, sibling) = (.shift, 0x2, 0x4, 60)
        case 60: (flag, device, siblingDevice, sibling) = (.shift, 0x4, 0x2, 56)
        case 59: (flag, device, siblingDevice, sibling) = (.control, 0x1, 0x2000, 62)
        case 62: (flag, device, siblingDevice, sibling) = (.control, 0x2000, 0x1, 59)
        case 58: (flag, device, siblingDevice, sibling) = (.option, 0x20, 0x40, 61)
        case 61: (flag, device, siblingDevice, sibling) = (.option, 0x40, 0x20, 58)
        case 55: (flag, device, siblingDevice, sibling) = (.command, 0x8, 0x10, 54)
        case 54: (flag, device, siblingDevice, sibling) = (.command, 0x10, 0x8, 55)
        case 57: return flags & NSEvent.ModifierFlags.capsLock.rawValue != 0
        default: return nil
        }
        guard flags & flag.rawValue != 0 else { return false }
        if flags & (device | siblingDevice) != 0 { return flags & device != 0 }
        // 沒有左右位元：兩邊都按著時又收到這個鍵，才是放開這一邊（另一邊還按著所以旗標還在）
        return !(down.contains(keyCode) && down.contains(sibling))
    }

    // WezTerm clears its inline IME buffer before interpreting each key-down.
    // Candidate navigation and repeated syllable keys can leave the snapshot identical.
    static func shouldSendMarkedText(_ result: EngineResult, appID: String, handledKeyDown: Bool,
                                     marked: Bool, previousText: String, cursor: Int,
                                     target: NSRange, replacement: NSRange) -> Bool {
        guard !result.composition.text.isEmpty else { return false }
        let refresh = handledKeyDown && appID.caseInsensitiveCompare("com.github.wez.wezterm") == .orderedSame
        return refresh || !marked || result.composition.text != previousText || result.composition.cursor != cursor ||
            result.composition.target != target || replacement.location != NSNotFound
    }

    // WezTerm consumes only the string. Send plain text with an end caret, and
    // invalidate an identical marked value before replaying it through IMK.
    static func writeMarkedText(_ result: EngineResult, appID: String, repeatText: Bool, replacement: NSRange,
                                send: (Any, NSRange, NSRange) -> Void) {
        let length = result.composition.text.utf16.count
        if appID.caseInsensitiveCompare("com.github.wez.wezterm") == .orderedSame {
            if repeatText && replacement.location == NSNotFound {
                send("", NSRange(location: 0, length: 0), replacement)
            }
            send(result.composition.text, NSRange(location: length, length: 0), replacement)
            return
        }
        let value = NSMutableAttributedString(string: result.composition.text, attributes: [
            .underlineStyle: NSUnderlineStyle.single.rawValue
        ])
        if result.composition.target.length > 0 && NSMaxRange(result.composition.target) <= length {
            value.addAttribute(.underlineStyle, value: NSUnderlineStyle.thick.rawValue, range: result.composition.target)
        }
        send(value, NSRange(location: min(result.composition.cursor, length), length: 0), replacement)
    }

    /// 組字游標在螢幕上的位置（`cursor` 從組字區開頭算，`marked` 是組字區在文件裡的範圍）。
    /// - `attributes(forCharacterIndex:)` 一般從組字區開頭算；Chromium 系 App（Edge、Discord、T3 Code、VS Code…）
    ///   卻當成整份文件的位置，而且只回答組字區裡的字。組字區不在文件開頭時，用「開頭量不到、文件位置量得到」認出來
    /// - 游標在組字區最後面時很多 App 量不到（超出文字範圍）：用最後一個字推算它的右緣
    /// - 不用 `firstRect(forCharacterRange:)`：經過 IMK 轉送回來的是垃圾值（1e-314、244497）
    /// 都量不到回傳 nil，呼叫端沿用按鍵前的位置
    static func measureCaret(cursor: Int, marked: NSRange, attributes: (Int) -> NSRect) -> NSRect? {
        let usable = { (rect: NSRect) -> NSRect? in
            let sane = [rect.minX, rect.minY, rect.width, rect.height].allSatisfy { $0.isFinite && abs($0) < 100_000 }
            return sane && rect.height >= 1 ? rect : nil
        }
        var base = 0
        if marked.location != NSNotFound && marked.location > 0 && usable(attributes(0)) == nil && usable(attributes(marked.location)) != nil {
            base = marked.location
        }
        if let rect = usable(attributes(base + cursor)) { return rect }
        guard cursor > 0, let last = usable(attributes(base + cursor - 1)) else { return nil }
        // 有些 App 回報的寬度只有 1（不是字寬）：用前兩個字的間距推算最後一個字的寬度
        var width = last.width
        if width <= 1, cursor > 1, let before = usable(attributes(base + cursor - 2)), before.minY == last.minY, last.minX > before.minX {
            width = last.minX - before.minX
        }
        return NSRect(x: last.minX + width, y: last.minY, width: 1, height: last.height)
    }

    private func render(_ result: EngineResult, client: IMKTextInput, handledKeyDown: Bool = false) {
        if !result.aiPending { aiTimer?.invalidate() }
        mode = result.mode; game = result.game; simplified = result.simplified; showComposition = result.compositionBar
        if !result.commit.isEmpty {
            client.insertText(result.commit, replacementRange: replacement)
            replacement = NSRange(location: NSNotFound, length: 0)
            marked = false; markedCursor = -1
        }
        if Self.shouldSendMarkedText(result, appID: appID, handledKeyDown: handledKeyDown,
                                     marked: marked, previousText: currentPreedit, cursor: markedCursor,
                                     target: markedTarget, replacement: replacement) {
            Self.writeMarkedText(result, appID: appID, repeatText: handledKeyDown && marked && result.composition.text == currentPreedit,
                                 replacement: replacement) { text, selection, range in
                client.setMarkedText(text, selectionRange: selection, replacementRange: range)
            }
            replacement = NSRange(location: NSNotFound, length: 0)
            marked = true
            markedCursor = result.composition.cursor
            markedTarget = result.composition.target
        } else if result.composition.text.isEmpty && marked {
            client.setMarkedText("", selectionRange: NSRange(location: 0, length: 0),
                                 replacementRange: NSRange(location: NSNotFound, length: 0))
            marked = false; markedCursor = -1
        }
        currentPreedit = result.composition.text
        if result.bell { NSSound.beep() }
        let cursor = result.composition.text.isEmpty ? 0 : result.composition.cursor
        let measured = measureCaret(client: client, cursor: cursor)
        // 量不到：沿用按鍵前量到的游標，都沒有才用滑鼠位置（當成沿用的位置，App 回報後再移過去）
        let fresh = measured != nil
        let caret = measured ?? knownCaret ?? NSRect(origin: NSEvent.mouseLocation, size: NSSize(width: 1, height: 18))
        if panel == nil {
            panel = CandidatePanel()
            panel?.onChoose = { [weak self] page, row in
                guard let self, let client = self.client(), let engine = self.engine else { return }
                do { self.render(try engine.choose(page: page, row: row), client: client) }
                catch { NSSound.beep() }
            }
            panel?.onPage = { [weak self] next in self?.performCommand(next ? 3 : 4) }
        }
        let plan = engine?.popupPlan(result, compositionBar: showComposition, caret: caret, fresh: fresh)
            ?? EngineSession.popupPlan(nil, result, compositionBar: showComposition, caret: caret, fresh: fresh)
        panel?.show(result: result, plan: plan, theme: theme)
    }

    private func measureCaret(client: IMKTextInput, cursor: Int) -> NSRect? {
        let marked = self.marked ? client.markedRange() : NSRange(location: NSNotFound, length: 0)
        var asked: [String] = []
        let caret = Self.measureCaret(cursor: cursor, marked: marked,
            attributes: { index in
                var rect = NSRect.zero; _ = client.attributes(forCharacterIndex: index, lineHeightRectangle: &rect)
                asked.append("attributes(\(index))=\(NSStringFromRect(rect))"); return rect
            })
        if let caret { knownCaret = caret }
        if Self.debugKeys {
            // 只記位置數字（不含打字內容）；標成 public，Console 才看得到
            let line = "\(appID) cursor=\(cursor) marked=\(marked.location == NSNotFound ? -1 : marked.location)+\(marked.length) "
                + asked.joined(separator: " ") + " -> " + (caret.map { NSStringFromRect($0) } ?? "量不到")
            Self.caretLog.notice("ZooType caret \(line, privacy: .public)")
        }
        return caret
    }

    private func pollAI(_ session: EngineSession) {
        aiTimer?.invalidate()
        aiTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self, weak session] timer in
            guard let self, let session, self.engine === session, let client = self.client() else { timer.invalidate(); return }
            do { if let result = try session.poll() { self.render(result, client: client); timer.invalidate() } }
            catch { timer.invalidate(); NSLog("ZooType AI: %@", String(describing: error)) }
        }
    }
    /// 按鍵診斷（資料資料夾的 local/ 下有 debug-keys 檔案時，記在系統記錄，用 Console 看「ZooType key」）：
    /// 和 Windows 一樣不記打的內容：一般字元鍵只記「字元鍵」，修飾鍵與功能鍵記鍵碼
    private static let debugKeys: Bool = {
        guard let config = appRuntime?.configURL else { return false }
        return FileManager.default.fileExists(atPath: config.deletingLastPathComponent().appendingPathComponent("local/debug-keys").path)
    }()
    private static let caretLog = Logger(subsystem: Bundle.main.bundleIdentifier ?? "ZooType", category: "diagnostics")
    private static let functionKeys: Set<UInt16> = [36, 48, 49, 51, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 96, 97, 98, 99, 100, 101, 103,
                                                     109, 111, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126]
    private static func debugKey(_ event: NSEvent, mods: UInt32, released: Bool, result: EngineResult) {
        guard debugKeys else { return }
        let key = functionKeys.contains(event.keyCode) ? "鍵碼 \(event.keyCode)" : "字元鍵"
        let kind = event.type == .flagsChanged ? "flagsChanged" : event.type == .keyDown ? "keyDown" : "keyUp"
        let line = "\(kind) \(key) \(released ? "放開" : "按下") mods=\(mods) t=\(Int(event.timestamp * 1000)) eaten=\(result.eaten ? "y" : "n") mode=\(result.mode)"
        caretLog.notice("ZooType key \(line, privacy: .public)")
    }

    /// 終端機清單在共用層（與 Windows 同一份）
    static func isTerminal(_ bundleID: String) -> Bool { bundleID.withCString { zct_is_terminal($0) } }

    private func applySettings(_ snapshot: [String: Any]) {
        let document = snapshot["document"] as? [String: Any]
        let config = document?["config"] as? [String: Any] ?? [:]
        theme = CandidateTheme(config: config)
    }
    private func refreshIdleSettings() {
        // 共用層判斷設定有沒有變、能不能換（組字中等組字結束）；換了才重新讀外觀
        guard let engine, let runtime = appRuntime, let client = client(), engine.refresh() else { return }
        do {
            let snapshot = try runtime.preferences(["op": "load"])["snapshot"] as? [String: Any] ?? [:]
            applySettings(snapshot)
            render(try engine.command(5), client: client)
        } catch { NSLog("ZooType settings reload: %@", String(describing: error)) }
    }
    private func performCommand(_ code: UInt32) {
        guard let engine, let client = client() else { return }
        do { render(try engine.command(code), client: client) }
        catch { NSLog("ZooType menu: %@", String(describing: error)) }
    }

    private func reconvert(client: IMKTextInput) {
        let range = client.selectedRange()
        guard range.location != NSNotFound, range.length > 0, range.length <= 256,
              let selected = client.attributedSubstring(from: range)?.string,
              selected.utf16.count == range.length, let engine else { NSSound.beep(); return }
        do {
            let result = try engine.reconvert(selected)
            guard !result.composition.text.isEmpty else { NSSound.beep(); return }
            replacement = range
            render(result, client: client)
        } catch { NSLog("ZooType reconvert: %@", String(describing: error)) }
    }

    override func commitComposition(_ sender: Any!) {
        guard let engine, let client = (sender as? IMKTextInput) ?? client() else { return }
        do { render(try engine.flush(), client: client) }
        catch {
            NSLog("ZooType commit: %@", String(describing: error))
            if !currentPreedit.isEmpty { client.insertText(currentPreedit, replacementRange: replacement) }
            marked = false
            currentPreedit = ""
        }
        panel?.orderOut(nil)
    }

    /// 選單內容與 Windows 共用（zct_core::menu）：這裡只照清單畫
    override func menu() -> NSMenu! {
        let menu = NSMenu(title: "動物園輸入法")
        menu.autoenablesItems = false
        let raw = zct_menu_items(appRuntime?.handle, mode == 0, simplified, game)
        guard let raw else { return menu }
        defer { zct_string_free(raw) }
        let items = (try? JSONSerialization.jsonObject(with: Data(String(cString: raw).utf8))) as? [[String: Any]] ?? []
        let actions: [String: Selector] = [
            "open_about": #selector(openAbout(_:)), "check_update": #selector(checkUpdate(_:)),
            "toggle_mode": #selector(toggleMode(_:)),
            "toggle_output": #selector(toggleOutput(_:)), "toggle_game": #selector(toggleGame(_:)),
            "open_settings": #selector(editSettings(_:)), "restart": #selector(restart(_:))
        ]
        for item in items {
            guard item["kind"] as? String == "action", let name = item["action"] as? String, let selector = actions[name] else {
                menu.addItem(.separator()); continue
            }
            let entry = menu.addItem(withTitle: item["label"] as? String ?? "", action: selector, keyEquivalent: "")
            entry.target = self
            entry.state = item["checked"] as? Bool == true ? .on : .off
            entry.isEnabled = item["enabled"] as? Bool ?? true
        }
        return menu
    }

    @objc private func settingsDidChange(_ sender: Any?) { refreshIdleSettings() }
    @objc private func prepareUpdate(_ sender: Any?) { commitComposition(client()); aiTimer?.invalidate() }
    // 檢查更新的結果與「有新版本」都在設定的「關於」頁
    @objc private func openAbout(_ sender: Any?) { openSettings(page: "about") }
    @objc private func checkUpdate(_ sender: Any?) { UpdateManager.shared.check(); openSettings(page: "about") }
    @objc private func toggleMode(_ sender: Any?) { performCommand(0) }
    @objc private func toggleOutput(_ sender: Any?) { performCommand(1) }
    @objc private func toggleGame(_ sender: Any?) { performCommand(2) }

    @objc private func editSettings(_ sender: Any?) { openSettings(page: nil) }

    /// 開啟設定；`page` 是設定頁的 id（例如 about）
    private func openSettings(page: String?) {
        NotificationCenter.default.post(name: .zctOpenSettings, object: page)
    }

    override func showPreferences(_ sender: Any!) { editSettings(sender) }

    /// 重新啟動：存檔、重新開啟引擎（重讀詞庫）、重建 session 並重讀設定
    @objc private func restart(_ sender: Any?) {
        guard let client = client() else { return }
        commitComposition(client)
        if let runtime = appRuntime {
            runtime.save()
            do { try runtime.recover() } catch { NSLog("ZooType restart: %@", String(describing: error)) }
        }
        activateServer(client)
    }
}
