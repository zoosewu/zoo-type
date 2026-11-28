import AppKit

extension Notification.Name {
    static let zctUpdateChanged = Notification.Name("ZooTypeUpdateChanged")
    static let zctWillInstallUpdate = Notification.Name("ZooTypeWillInstallUpdate")
}

enum UpdateFiles {
    #if arch(arm64)
    static let architecture = "arm64"
    #else
    static let architecture = "x64"
    #endif
    @discardableResult
    static func run(_ executable: String, _ arguments: [String]) throws -> String {
        let process = Process(); process.executableURL = URL(fileURLWithPath: executable); process.arguments = arguments
        let output = Pipe(); process.standardOutput = output; process.standardError = output
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile(); process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw EngineError(description: "更新操作失敗：" + String(decoding: data, as: UTF8.self).prefix(1500)) }
        return String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    }
    static func validateApp(_ app: URL, version: String, bundleID: String) throws {
        let data = try Data(contentsOf: app.appendingPathComponent("Contents/Info.plist"))
        let metadata = try PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any]
        guard metadata?["CFBundleIdentifier"] as? String == bundleID, metadata?["ZooTypeVersion"] as? String == version,
              metadata?["CFBundleExecutable"] as? String == "zoo-type" else { throw EngineError(description: "更新 App 的身分或版本不符") }
        try run("/usr/bin/codesign", ["--verify", "--strict", app.path])
        let arch = try run("/usr/bin/lipo", ["-archs", app.appendingPathComponent("Contents/MacOS/zoo-type").path])
        guard arch.split(separator: " ").contains(architecture == "arm64" ? "arm64" : "x86_64") else { throw EngineError(description: "更新 App 的處理器架構不符") }
    }
    // A staged copy and rollback are on the destination filesystem. No shell paths.
    // 暫存與舊版放在 Input Methods 外面：留在裡面的每一份 .app 都會被系統當成另一個輸入法。
    // 回傳的舊版所在資料夾由呼叫端在新版啟動後刪除。
    static func replaceApp(staged: URL, installed: URL) throws -> URL {
        let manager = FileManager.default
        let stage = try manager.url(for: .itemReplacementDirectory, in: .userDomainMask, appropriateFor: installed, create: true)
        let copy = stage.appendingPathComponent("new.app"), backup = stage.appendingPathComponent("previous.app")
        do {
            try manager.copyItem(at: staged, to: copy)
            try manager.moveItem(at: installed, to: backup)
        } catch { try? manager.removeItem(at: stage); throw error }
        do { try manager.moveItem(at: copy, to: installed) }
        catch {
            do { try manager.moveItem(at: backup, to: installed) }
            catch let rollback { throw EngineError(description: "更新失敗且無法還原舊版：" + String(describing: rollback)) }
            try? manager.removeItem(at: stage)
            throw error
        }
        return backup
    }
    // 0.1.7 以前的安裝腳本與自動更新把舊版留在 Input Methods 裡（<名稱>.app.previous-*）
    static func removeLeftovers(beside app: URL) {
        let manager = FileManager.default, folder = app.deletingLastPathComponent(), prefix = app.lastPathComponent + ".previous-"
        for name in (try? manager.contentsOfDirectory(atPath: folder.path)) ?? [] where name.hasPrefix(prefix) {
            try? manager.removeItem(at: folder.appendingPathComponent(name))
        }
    }
}

// 查、下載、驗證、排程與顯示的用詞在共用層（與 Windows 相同，依 zoo-release 的 UPDATE.md／UI.md）；這裡只負責安裝（解壓、驗 App、替換、重新啟動）。
// All state belongs to the main thread. Extraction uses a background queue.
final class UpdateManager {
    static let shared = UpdateManager()
    private(set) var message = "尚未檢查更新"
    private(set) var available: String?
    private(set) var busy = false
    private(set) var supported = false
    private var checking = false
    private var installing = false
    /// 安裝的結果等平台自己的訊息：保留到下次檢查，不被共用層的狀態蓋掉
    private var local: String?
    private weak var runtime: EngineRuntime?
    private var timer: Timer?
    private let queue = DispatchQueue(label: "ZooType.updates", qos: .utility)
    func start(runtime: EngineRuntime) {
        self.runtime = runtime
        refresh()
        if !supported { message = "開發版不提供自動更新"; return }
        timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in self?.refresh() }
    }
    private func refresh() {
        guard let runtime else { return }
        let status = zct_update_status(runtime.handle)
        supported = status.supported
        guard supported else { return }
        let text = String(cString: status.message)
        let version = String(cString: status.available)
        checking = status.checking
        let next = (local ?? text, version.isEmpty ? nil : version, status.busy || installing)
        guard next.0 != message || next.1 != available || next.2 != busy else { return }
        (message, available, busy) = next
        NotificationCenter.default.post(name: .zctUpdateChanged, object: nil)
    }
    private func changed(_ message: String) {
        self.message = message
        local = message
        NotificationCenter.default.post(name: .zctUpdateChanged, object: nil)
    }
    func check() {
        guard supported, !busy, let runtime else { return }
        local = nil
        if !zct_update_check(runtime.handle) { changed("無法檢查更新"); return }
        refresh()
    }
    func cancel() {
        guard let runtime else { return }
        zct_update_cancel(runtime.handle)
    }
    /// 「關於」頁的更新區塊：狀態列（文字、次要文字、下載進度 0–100）、按鈕（id：notes／install／cancel／retry／manual）、Release 頁面
    func view() -> (text: String, detail: String, progress: Int?, buttons: [(id: String, label: String)], page: String) {
        guard let runtime, let raw = zct_update_view(runtime.handle) else { return ("", "", nil, [], "") }
        defer { zct_string_free(raw) }
        let json = (try? JSONSerialization.jsonObject(with: Data(String(cString: raw).utf8))) as? [String: Any] ?? [:]
        let text = local ?? json["text"] as? String ?? ""
        let buttons = (json["buttons"] as? [[String: Any]] ?? []).compactMap { b -> (String, String)? in
            guard let id = b["id"] as? String, let label = b["label"] as? String else { return nil }
            return (id, label)
        }
        // 這裡自己記的訊息（無法檢查更新等）取代狀態列時，不帶次要文字與進度
        let detail = local == nil ? json["detail"] as? String ?? "" : ""
        let progress = local == nil ? (json["progress"] as? NSNumber)?.intValue : nil
        return (text, detail, progress, buttons, json["release_page"] as? String ?? "")
    }
    /// 作者的其他專案（apex-code API）；讀取失敗是空陣列（整個區塊不顯示）
    func otherProjects(_ done: @escaping ([[String: Any]]) -> Void) {
        guard let runtime else { done([]); return }
        let handle = runtime.handle
        queue.async {
            var list: [[String: Any]] = []
            if let raw = zct_other_projects(handle) {
                list = (try? JSONSerialization.jsonObject(with: Data(String(cString: raw).utf8))) as? [[String: Any]] ?? []
                zct_string_free(raw)
            }
            DispatchQueue.main.async { done(list) }
        }
    }
    func install() {
        guard supported, !busy, let version = available, let runtime else { return }
        let installed = Bundle.main.bundleURL.standardizedFileURL
        let expected = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Input Methods/動物園輸入法.app").standardizedFileURL
        guard installed == expected else { changed("請先將正式版安裝至 Library/Input Methods，再使用自動更新"); return }
        // 按下才下載：共用層在背景下載並驗證簽章與版號（進度在共用層的狀態裡）
        local = nil
        installing = true; busy = true
        refresh()
        let bundleID = Bundle.main.bundleIdentifier!
        let handle = runtime.handle
        queue.async { [weak self] in
            var work: URL?
            // 下載失敗、取消時共用層已經更新狀態，這裡不再另外標記失敗
            var downloaded = false
            let result = Result { () throws -> URL in
                guard let pending = zct_update_prepare(handle) else { throw EngineError.current }
                downloaded = true
                let file = URL(fileURLWithPath: String(cString: pending)); zct_string_free(pending)
                let staging = file.deletingLastPathComponent().appendingPathComponent("staged-" + UUID().uuidString)
                work = staging
                try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
                try UpdateFiles.run("/usr/bin/ditto", ["-x", "-k", file.path, staging.path])
                let app = staging.appendingPathComponent("zoo-type-\(version)-macos-\(UpdateFiles.architecture)/動物園輸入法.app")
                try UpdateFiles.validateApp(app, version: version, bundleID: bundleID)
                return app
            }
            DispatchQueue.main.async {
                defer { if let work { try? FileManager.default.removeItem(at: work) } }
                guard let self else { return }
                defer {
                    self.installing = false; self.busy = self.checking
                    zct_update_installing(runtime.handle, nil)
                }
                var frozen = false
                do {
                    let staged = try result.get()
                    version.withCString { zct_update_installing(runtime.handle, $0) }
                    NotificationCenter.default.post(name: .zctWillInstallUpdate, object: nil)
                    try runtime.freeze()
                    frozen = true
                    let backup = try UpdateFiles.replaceApp(staged: staged, installed: installed)
                    let process = Process(); process.executableURL = URL(fileURLWithPath: "/bin/sh")
                    let restart = """
                    while kill -0 "$1" 2>/dev/null; do sleep 1; done
                    if /usr/bin/open -g -n "$2"; then /bin/rm -rf "${3%/*}"; exit 0; fi
                    # 換回舊版成功才刪暫存，否則舊版只剩暫存裡那一份
                    /bin/mv "$2" "${3%/*}/failed.app" && /bin/mv "$3" "$2" || exit 1
                    /bin/rm -rf "${3%/*}"
                    /usr/bin/open -g -n "$2"
                    """
                    process.arguments = ["-c", restart, "zoo-type-update", String(ProcessInfo.processInfo.processIdentifier), installed.path, backup.path]
                    do { try process.run() }
                    catch {
                        try FileManager.default.removeItem(at: installed)
                        try FileManager.default.moveItem(at: backup, to: installed)
                        try? FileManager.default.removeItem(at: backup.deletingLastPathComponent())
                        throw error
                    }
                    self.changed("更新完成，輸入法重新啟動；若未切換，請登出再登入")
                    NSApp.terminate(nil)
                } catch {
                    if frozen {
                        do { try runtime.recover() } catch { self.changed("更新失敗，請重新啟動輸入法：" + String(describing: error)); return }
                        NotificationCenter.default.post(name: .zctSettingsChanged, object: nil)
                    }
                    if downloaded { zct_update_failed(runtime.handle, String(describing: error)) }
                    self.refresh()
                }
            }
        }
    }
}
