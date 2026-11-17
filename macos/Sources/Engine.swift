import Foundation

struct EngineResult {
    let eaten: Bool
    let commit: String
    let preedit: String
    let cursor: Int
    let target: NSRange
    let candidates: [String]
    let pinned: [Bool]
    let columns: [[String]]
    let firstColumn: Int
    let highlight: Int
    let page: Int
    let totalPages: Int
    let notice: String
    let bell: Bool
    let reconvert: Bool
    let mode: UInt32

    init(_ value: ZctResult) throws {
        guard value.ok else { throw EngineError.current }
        func text(_ p: UnsafePointer<CChar>?) -> String { p.map { String(cString: $0) } ?? "" }
        eaten = value.eaten
        commit = text(value.commit)
        preedit = text(value.preedit)
        cursor = Int(value.cursor)
        target = NSRange(location: Int(value.target_start), length: Int(value.target_length))
        candidates = (0..<Int(value.candidate_count)).map { text(value.candidates[$0]) }
        pinned = (0..<Int(value.candidate_count)).map { value.pinned[$0] }
        columns = (0..<Int(value.column_count)).map { column in
            let view = value.columns[column]
            return (0..<Int(view.count)).map { text(view.items[$0]) }
        }
        firstColumn = Int(value.first_column)
        highlight = Int(value.highlight)
        page = Int(value.page)
        totalPages = Int(value.total_pages)
        notice = text(value.notice)
        bell = value.bell
        reconvert = value.reconvert
        mode = value.mode
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
    func save() { if !zct_runtime_save(handle) { NSLog("ZooType save: %@", EngineError.current.description) } }
    deinit { zct_runtime_free(handle) }
}

final class EngineSession {
    let runtime: EngineRuntime
    let handle: OpaquePointer
    init(runtime: EngineRuntime, app: String, previousMode: UInt32? = nil) throws {
        self.runtime = runtime
        guard let handle = app.withCString({ zct_session_new(runtime.handle, $0) }) else { throw EngineError.current }
        self.handle = handle
        if let mode = previousMode { zct_session_restore_mode(handle, mode) }
    }
    func key(code: UInt16, modifiers: UInt32, released: Bool = false, time: UInt64) throws -> EngineResult {
        try EngineResult(zct_session_key(handle, code, modifiers, released, time))
    }
    func flush() throws -> EngineResult { try EngineResult(zct_session_flush(handle)) }
    func reconvert(_ text: String) throws -> EngineResult {
        try text.withCString { try EngineResult(zct_session_reconvert(handle, $0)) }
    }
    deinit { zct_session_free(handle) }
}
