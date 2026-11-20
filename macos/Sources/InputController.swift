import AppKit
import InputMethodKit

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
    private var settingsRevision = ""
    private var detectFullscreen = true
    private var compositionAnchor: NSRect?
    private var game = false
    private var simplified = false
    private var replacement = NSRange(location: NSNotFound, length: 0)
    private static var modes: [String: UInt32] = [:]

    override func recognizedEvents(_ sender: Any!) -> Int {
        Int(NSEvent.EventTypeMask.keyDown.rawValue | NSEvent.EventTypeMask.keyUp.rawValue |
            NSEvent.EventTypeMask.flagsChanged.rawValue | NSEvent.EventTypeMask.leftMouseDown.rawValue |
            NSEvent.EventTypeMask.rightMouseDown.rawValue)
    }

    override func activateServer(_ sender: Any!) {
        guard let client = (sender as? IMKTextInput) ?? client() else { return }
        if engine != nil {
            commitComposition(sender)
            Self.modes[appID] = mode
        }
        let clientID = client.bundleIdentifier() ?? ""
        appID = clientID.isEmpty ? (NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "") : clientID
        NotificationCenter.default.removeObserver(self, name: .zctSettingsChanged, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(settingsDidChange(_:)), name: .zctSettingsChanged, object: nil)
        NotificationCenter.default.removeObserver(self, name: .zctWillInstallUpdate, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(prepareUpdate(_:)), name: .zctWillInstallUpdate, object: nil)
        modifiersDown.removeAll()
        replacement = NSRange(location: NSNotFound, length: 0)
        do {
            guard let runtime = appRuntime else { throw EngineError(description: "輸入引擎未啟動，請查看 Console 的 ZooType 記錄") }
            engine = try EngineSession(runtime: runtime, app: appID, previousMode: Self.modes[appID])
            let state = try engine!.command(5)
            mode = state.mode
            simplified = state.simplified
            game = state.game
            let snapshot = try runtime.preferences(["op": "load"])["snapshot"] as? [String: Any] ?? [:]
            applySettings(snapshot)
            settingsTimer?.invalidate()
            settingsTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.refreshIdleSettings() }
        } catch { NSLog("ZooType activate: %@", String(describing: error)); NSSound.beep() }
    }

    override func deactivateServer(_ sender: Any!) {
        commitComposition(sender)
        Self.modes[appID] = mode
        aiTimer?.invalidate(); settingsTimer?.invalidate()
        compositionAnchor = nil
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
        let signals = InputDetection.gather(hasTextContext: hasTextContext, hasCaret: caret.height > 0)
        if InputDetection.shouldPass(signals, enabled: detectFullscreen) {
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
            let flag: NSEvent.ModifierFlags
            switch event.keyCode {
            case 56, 60: flag = .shift
            case 59, 62: flag = .control
            case 58, 61: flag = .option
            case 54, 55: flag = .command
            case 57: flag = .capsLock
            default: return false
            }
            released = !event.modifierFlags.contains(flag) || modifiersDown.contains(event.keyCode)
            if released { modifiersDown.remove(event.keyCode) } else { modifiersDown.insert(event.keyCode) }
        }
        if modifiersDown.contains(61) { mods |= 32 }
        do {
            var result = try engine.key(code: event.keyCode, modifiers: mods, released: released,
                                        time: UInt64(max(0, event.timestamp * 1000)))
            if result.toggleCompositionBar {
                showComposition.toggle()
                do { try persistCompositionBar(); result.notice = "組字提示列：" + (showComposition ? "開" : "關") }
                catch { showComposition.toggle(); result.notice = "無法儲存組字提示列：" + String(describing: error) }
            }
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

    private func render(_ result: EngineResult, client: IMKTextInput, handledKeyDown: Bool = false) {
        if !result.aiPending { aiTimer?.invalidate() }
        mode = result.mode; game = result.game; simplified = result.simplified
        if !result.commit.isEmpty {
            client.insertText(result.commit, replacementRange: replacement)
            replacement = NSRange(location: NSNotFound, length: 0)
            marked = false
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
            marked = false
        }
        currentPreedit = result.composition.text
        if result.bell { NSSound.beep() }
        var caret = NSRect.zero
        _ = client.attributes(forCharacterIndex: result.composition.text.isEmpty ? 0 : result.composition.cursor, lineHeightRectangle: &caret)
        if caret == .zero { caret = NSRect(origin: NSEvent.mouseLocation, size: NSSize(width: 1, height: 18)) }
        if panel == nil {
            panel = CandidatePanel()
            panel?.onChoose = { [weak self] page, row in
                guard let self, let client = self.client(), let engine = self.engine else { return }
                do { self.render(try engine.choose(page: page, row: row), client: client) }
                catch { NSSound.beep() }
            }
            panel?.onPage = { [weak self] next in self?.performCommand(next ? 3 : 4) }
        }
        if result.composition.text.isEmpty { compositionAnchor = nil }
        else if showComposition && compositionAnchor == nil { compositionAnchor = caret }
        if showComposition, let anchor = compositionAnchor { caret = anchor }
        panel?.show(result: result, caret: caret, theme: theme, showComposition: showComposition)
    }

    private func pollAI(_ session: EngineSession) {
        aiTimer?.invalidate()
        aiTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self, weak session] timer in
            guard let self, let session, self.engine === session, let client = self.client() else { timer.invalidate(); return }
            do { if let result = try session.poll() { self.render(result, client: client); timer.invalidate() } }
            catch { timer.invalidate(); NSLog("ZooType AI: %@", String(describing: error)) }
        }
    }
    private func persistCompositionBar() throws {
        guard let runtime = appRuntime, !appID.isEmpty else { throw EngineError(description: "無法辨識目前 App") }
        try PreferencesModel(client: runtime).saveCompositionBar(showComposition, for: appID)
    }
    static func isTerminal(_ bundleID: String) -> Bool {
        ["com.apple.Terminal", "com.googlecode.iterm2", "dev.warp.Warp-Stable", "com.github.wez.wezterm"]
            .contains { $0.caseInsensitiveCompare(bundleID) == .orderedSame }
    }

    private func applySettings(_ snapshot: [String: Any]) {
        let document = snapshot["document"] as? [String: Any]
        let config = document?["config"] as? [String: Any] ?? [:]
        let apps = (document?["apps"] as? [String: Any])?["app"] as? [[String: Any]] ?? []
        let rule = apps.first { $0["macos"] as? String == appID }
        theme = CandidateTheme(config: config)
        let fullscreen = (config["detect"] as? [String: Any])?["fullscreen"] as? Bool ?? true
        detectFullscreen = fullscreen && (rule?["detect"] as? Bool ?? true)
        showComposition = rule?["composition_bar"] as? Bool ?? (theme.compositionBar || Self.isTerminal(appID))
        settingsRevision = snapshot["revision"] as? String ?? ""
    }
    private func refreshIdleSettings() {
        guard let engine, engine.isIdle, let runtime = appRuntime, let client = client() else { return }
        do {
            let snapshot = try runtime.preferences(["op": "load"])["snapshot"] as? [String: Any] ?? [:]
            if snapshot["revision"] as? String != settingsRevision {
                let fresh = try EngineSession(runtime: runtime, app: appID)
                fresh.keepState(of: engine)
                self.engine = fresh
                applySettings(snapshot)
                render(try fresh.command(5), client: client)
            }
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

    override func menu() -> NSMenu! {
        let menu = NSMenu(title: "動物園輸入法")
        menu.autoenablesItems = false
        let modeItem = menu.addItem(withTitle: "中文模式", action: #selector(toggleMode(_:)), keyEquivalent: "")
        modeItem.state = mode == 0 ? .on : .off; modeItem.target = self
        let output = menu.addItem(withTitle: "簡體輸出", action: #selector(toggleOutput(_:)), keyEquivalent: "")
        output.state = simplified ? .on : .off; output.target = self
        let gameItem = menu.addItem(withTitle: "遊戲模式", action: #selector(toggleGame(_:)), keyEquivalent: "")
        gameItem.state = game ? .on : .off; gameItem.target = self
        menu.addItem(.separator())
        let settings = menu.addItem(withTitle: "設定…", action: #selector(editSettings(_:)), keyEquivalent: "")
        settings.target = self
        let reload = menu.addItem(withTitle: "重新載入設定", action: #selector(reloadSettings(_:)), keyEquivalent: "")
        reload.target = self
        if let version = UpdateManager.shared.available {
            let update = menu.addItem(withTitle: "更新到 v" + version, action: #selector(installUpdate(_:)), keyEquivalent: "")
            update.target = self; update.isEnabled = !UpdateManager.shared.busy
        }
        return menu
    }

    @objc private func settingsDidChange(_ sender: Any?) { settingsRevision = ""; refreshIdleSettings() }
    @objc private func prepareUpdate(_ sender: Any?) { commitComposition(client()); aiTimer?.invalidate() }
    @objc private func installUpdate(_ sender: Any?) { UpdateManager.shared.install() }
    @objc private func toggleMode(_ sender: Any?) { performCommand(0) }
    @objc private func toggleOutput(_ sender: Any?) { performCommand(1) }
    @objc private func toggleGame(_ sender: Any?) { performCommand(2) }

    @objc private func editSettings(_ sender: Any?) {
        NotificationCenter.default.post(name: .zctOpenSettings, object: nil)
    }

    override func showPreferences(_ sender: Any!) { editSettings(sender) }

    @objc private func reloadSettings(_ sender: Any?) {
        guard let client = client() else { return }
        activateServer(client)
    }
}
