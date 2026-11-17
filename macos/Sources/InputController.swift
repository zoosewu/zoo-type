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
        appID = client.bundleIdentifier() ?? ""
        modifiersDown.removeAll()
        replacement = NSRange(location: NSNotFound, length: 0)
        do {
            guard let runtime = appRuntime else { throw EngineError(description: "輸入引擎未啟動，請查看 Console 的 ZooType 記錄") }
            engine = try EngineSession(runtime: runtime, app: appID, previousMode: Self.modes[appID])
            mode = try engine!.flush().mode
            let config = try runtime.settings()
            theme = CandidateTheme(config: config)
            let rules = (try runtime.settings(apps: true)["app"] as? [[String: Any]]) ?? []
            let rule = rules.first { $0["macos"] as? String == appID }
            showComposition = rule?["composition_bar"] as? Bool ??
                (theme.compositionBar || ["com.apple.Terminal", "com.googlecode.iterm2", "dev.warp.Warp-Stable"].contains(appID))
        } catch { NSLog("ZooType activate: %@", String(describing: error)); NSSound.beep() }
    }

    override func deactivateServer(_ sender: Any!) {
        commitComposition(sender)
        Self.modes[appID] = mode
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
            let result = try engine.key(code: event.keyCode, modifiers: mods, released: released,
                                        time: UInt64(max(0, event.timestamp * 1000)))
            render(result, client: client)
            if result.reconvert { reconvert(client: client) }
            return event.type == .flagsChanged ? false : result.eaten
        } catch {
            NSLog("ZooType key: %@", String(describing: error))
            commitComposition(sender)
            return false
        }
    }

    private func render(_ result: EngineResult, client: IMKTextInput) {
        mode = result.mode
        if !result.commit.isEmpty {
            client.insertText(result.commit, replacementRange: replacement)
            replacement = NSRange(location: NSNotFound, length: 0)
            marked = false
        }
        if !result.preedit.isEmpty && (!marked || result.preedit != currentPreedit ||
            result.cursor != markedCursor || result.target != markedTarget || replacement.location != NSNotFound) {
            let value = NSMutableAttributedString(string: result.preedit, attributes: [
                .underlineStyle: NSUnderlineStyle.single.rawValue
            ])
            if result.target.length > 0 && NSMaxRange(result.target) <= value.length {
                value.addAttribute(.underlineStyle, value: NSUnderlineStyle.thick.rawValue, range: result.target)
            }
            client.setMarkedText(value, selectionRange: NSRange(location: min(result.cursor, value.length), length: 0),
                                 replacementRange: replacement)
            replacement = NSRange(location: NSNotFound, length: 0)
            marked = true
            markedCursor = result.cursor
            markedTarget = result.target
        } else if result.preedit.isEmpty && marked {
            client.setMarkedText("", selectionRange: NSRange(location: 0, length: 0),
                                 replacementRange: NSRange(location: NSNotFound, length: 0))
            marked = false
        }
        currentPreedit = result.preedit
        if result.bell { NSSound.beep() }
        var caret = NSRect.zero
        _ = client.attributes(forCharacterIndex: result.preedit.isEmpty ? 0 : result.cursor, lineHeightRectangle: &caret)
        if caret == .zero { caret = NSRect(origin: NSEvent.mouseLocation, size: NSSize(width: 1, height: 18)) }
        if panel == nil { panel = CandidatePanel() }
        panel?.show(result: result, caret: caret, theme: theme, showComposition: showComposition)
    }

    private func reconvert(client: IMKTextInput) {
        let range = client.selectedRange()
        guard range.location != NSNotFound, range.length > 0, range.length <= 256,
              let selected = client.attributedSubstring(from: range)?.string,
              selected.utf16.count == range.length, let engine else { NSSound.beep(); return }
        do {
            let result = try engine.reconvert(selected)
            guard !result.preedit.isEmpty else { NSSound.beep(); return }
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
        let settings = menu.addItem(withTitle: "設定…", action: #selector(editSettings(_:)), keyEquivalent: "")
        settings.target = self
        let reload = menu.addItem(withTitle: "重新載入設定", action: #selector(reloadSettings(_:)), keyEquivalent: "")
        reload.target = self
        return menu
    }

    @objc private func editSettings(_ sender: Any?) {
        NotificationCenter.default.post(name: .zctOpenSettings, object: nil)
    }

    override func showPreferences(_ sender: Any!) { editSettings(sender) }

    @objc private func reloadSettings(_ sender: Any?) {
        guard let client = client() else { return }
        activateServer(client)
    }
}
