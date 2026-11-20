import Foundation

struct CompositionDisplay {
    let text: String
    let cursor: Int
    let target: NSRange
}

struct CandidateDisplay {
    let items: [String]
    let labels: [String]
    var columns: [[String]] = []
    var firstColumn = 0
    let highlight: Int?
    let page: Int
    let totalPages: Int
    var displayOnly = false
}

struct EngineResult {
    let eaten: Bool
    let commit: String
    let composition: CompositionDisplay
    let candidatePage: CandidateDisplay
    var notice: String
    let bell: Bool
    let reconvert: Bool
    let mode: UInt32
    let aiPending: Bool
    let toggleCompositionBar: Bool
    /// 這個 App 目前要不要顯示組字提示列（共用層依 App 規則、終端機、全域設定決定；快捷鍵已寫回 apps.toml）
    let compositionBar: Bool
    let game: Bool
    let simplified: Bool

    init(_ value: ZctResult) throws {
        guard value.ok else { throw EngineError.current }
        func text(_ p: UnsafePointer<CChar>?) -> String { p.map { String(cString: $0) } ?? "" }
        eaten = value.eaten
        commit = text(value.commit)
        composition = CompositionDisplay(text: text(value.preedit), cursor: Int(value.cursor),
                                         target: NSRange(location: Int(value.target_start), length: Int(value.target_length)))
        let columns = (0..<Int(value.column_count)).map { column in
            let view = value.columns[column]
            return (0..<Int(view.count)).map { text(view.items[$0]) }
        }
        candidatePage = CandidateDisplay(
            items: (0..<Int(value.candidate_count)).map { text(value.candidates[$0]) },
            labels: (0..<Int(value.candidate_count)).map { text(value.labels[$0]) },
            columns: columns, firstColumn: Int(value.first_column),
            highlight: value.has_highlight ? Int(value.highlight) : nil,
            page: Int(value.page), totalPages: Int(value.total_pages), displayOnly: value.display_only)
        notice = text(value.notice)
        bell = value.bell
        reconvert = value.reconvert
        mode = value.mode
        aiPending = value.ai_pending
        toggleCompositionBar = value.toggle_composition_bar
        compositionBar = value.composition_bar
        game = value.game
        simplified = value.simplified
    }
}

struct EngineError: Error, CustomStringConvertible {
    let description: String
    static var current: EngineError { EngineError(description: String(cString: zct_last_error())) }
}

final class EngineRuntime: PreferencesClient {
    let handle: OpaquePointer
    init(dictionary: String) throws {
        guard let handle = dictionary.withCString({ zct_runtime_new($0) }) else { throw EngineError.current }
        self.handle = handle
    }
    var configURL: URL { URL(fileURLWithPath: String(cString: zct_runtime_config_path(handle))) }
    func settings(apps: Bool = false) throws -> [String: Any] {
        guard let p = zct_runtime_settings(handle, apps) else { throw EngineError.current }
        return try JSONSerialization.jsonObject(with: Data(String(cString: p).utf8)) as! [String: Any]
    }
    func saveSettings(_ value: [String: Any], apps: Bool = false) throws {
        let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
        let success = String(decoding: data, as: UTF8.self).withCString { zct_runtime_save_settings(handle, apps, $0) }
        if !success { throw EngineError.current }
        NotificationCenter.default.post(name: .zctSettingsChanged, object: nil)
    }
    func preferences(_ request: [String: Any]) throws -> [String: Any] {
        if request["op"] as? String == "manage", let action = request["action"] as? [String: Any] {
            switch action["op"] as? String {
            case "api_key_status": return ["kind": "management", "reply": ["kind": "api_key_status", "present": try Credentials.read() != nil]]
            case "write_api_key":
                try Credentials.write((action["key"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines))
                NotificationCenter.default.post(name: .zctSettingsChanged, object: nil)
                return ["kind": "management", "reply": ["kind": "done", "restart_required": false]]
            case "test_ai":
                let value = try AIConnection.test(config: action["config"] as? [String: Any] ?? [:], key: Credentials.read() ?? "")
                return ["kind": "management", "reply": ["kind": "ai_test", "message": value]]
            default: break
            }
        }
        let data = try JSONSerialization.data(withJSONObject: request, options: [.sortedKeys])
        guard let result = String(decoding: data, as: UTF8.self).withCString({ zct_runtime_preferences(handle, $0) }) else {
            throw EngineError.current
        }
        return try JSONSerialization.jsonObject(with: Data(String(cString: result).utf8)) as! [String: Any]
    }
    func relocatePreferences(to path: String) throws -> [String: Any] {
        guard let result = path.withCString({ zct_runtime_relocate(handle, $0) }) else { throw EngineError.current }
        return try JSONSerialization.jsonObject(with: Data(String(cString: result).utf8)) as! [String: Any]
    }
    func recover() throws { guard zct_runtime_recover(handle) else { throw EngineError.current } }
    func freeze() throws { guard zct_runtime_freeze(handle) else { throw EngineError.current } }
    func save() { if !zct_runtime_save(handle) { NSLog("ZooType save: %@", EngineError.current.description) } }
    deinit { zct_runtime_free(handle) }
}

final class EngineSession {
    let runtime: EngineRuntime
    let handle: OpaquePointer
    /// 同一個 App 再次啟用時，共用層會沿用上次的中英狀態
    init(runtime: EngineRuntime, app: String) throws {
        self.runtime = runtime
        guard let handle = app.withCString({ zct_session_new(runtime.handle, $0) }) else { throw EngineError.current }
        self.handle = handle
        guard zct_session_ai_enabled(handle) else { return }
        do {
            let key = try Credentials.read() ?? ""
            guard key.withCString({ zct_session_ai_key(handle, $0) }) else { throw EngineError.current }
        } catch { NSLog("ZooType credentials unavailable: %@", String(describing: error)) }
    }
    func key(code: UInt16, modifiers: UInt32, released: Bool = false, time: UInt64) throws -> EngineResult {
        try EngineResult(zct_session_key(handle, code, modifiers, released, time))
    }
    func flush() throws -> EngineResult { try EngineResult(zct_session_flush(handle)) }
    func reconvert(_ text: String) throws -> EngineResult {
        try text.withCString { try EngineResult(zct_session_reconvert(handle, $0)) }
    }
    var isIdle: Bool { zct_session_is_idle(handle) }
    /// 不在輸入狀態（按鍵交給 App）：規則與 Windows 共用，依這個 App 的設定與規則
    func notInput(_ s: InputSignals) -> Bool { zct_session_not_input(handle, s.secure, s.hasTextContext, s.hasCaret, s.fullscreen) }
    /// 設定變了而且沒有組字：共用層換成新設定（沿用中英等狀態）。回傳有沒有換
    func refresh() -> Bool { zct_session_refresh(handle) }
    func command(_ code: UInt32) throws -> EngineResult { try EngineResult(zct_session_command(handle, code)) }
    func choose(page: Int, row: Int) throws -> EngineResult { try EngineResult(zct_session_choose(handle, UInt(max(0, page)), UInt(max(0, row)))) }
    func poll() throws -> EngineResult? {
        var result = ZctResult()
        guard zct_session_poll(handle, &result) else { return nil }
        return try EngineResult(result)
    }
    deinit { zct_session_free(handle) }
}
