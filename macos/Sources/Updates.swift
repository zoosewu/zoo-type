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
    static func replaceApp(staged: URL, installed: URL) throws -> URL {
        let manager = FileManager.default
        let stage = installed.deletingLastPathComponent().appendingPathComponent(".zoo-type-update-" + UUID().uuidString)
        try manager.createDirectory(at: stage, withIntermediateDirectories: true)
        defer { try? manager.removeItem(at: stage) }
        let copy = stage.appendingPathComponent(installed.lastPathComponent)
        try manager.copyItem(at: staged, to: copy)
        let backup = installed.appendingPathExtension("previous-" + UUID().uuidString)
        try manager.moveItem(at: installed, to: backup)
        do { try manager.moveItem(at: copy, to: installed) }
        catch {
            do { try manager.moveItem(at: backup, to: installed) }
            catch let rollback { throw EngineError(description: "更新失敗且無法還原舊版：" + String(describing: rollback)) }
            throw error
        }
        return backup
    }
}

// 查、下載、驗證、排程在共用層（與 Windows 相同）；這裡只顯示狀態與安裝（解壓、驗 App、替換、重新啟動）。
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
        if !zct_update_check(runtime.handle) { changed("檢查更新失敗：" + EngineError.current.description); return }
        refresh()
    }
    func install() {
        guard supported, !busy, let version = available, let runtime else { return }
        let installed = Bundle.main.bundleURL.standardizedFileURL
        let expected = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Input Methods/動物園輸入法.app").standardizedFileURL
        guard installed == expected else { changed("請先將正式版安裝至 Library/Input Methods，再使用自動更新"); return }
        // 共用層重新驗證過簽章與版號的檔案
        guard let pending = zct_update_pending(runtime.handle) else { changed("更新失敗：" + EngineError.current.description); return }
        let file = URL(fileURLWithPath: String(cString: pending))
        let directory = file.deletingLastPathComponent()
        installing = true; busy = true; changed("準備安裝…")
        zct_update_installing(runtime.handle, true)
        let bundleID = Bundle.main.bundleIdentifier!
        queue.async { [weak self] in
            let work = directory.appendingPathComponent("staged-" + UUID().uuidString)
            let result = Result { () throws -> URL in
                try FileManager.default.createDirectory(at: work, withIntermediateDirectories: true)
                try UpdateFiles.run("/usr/bin/ditto", ["-x", "-k", file.path, work.path])
                let app = work.appendingPathComponent("zoo-type-\(version)-macos-\(UpdateFiles.architecture)/動物園輸入法.app")
                try UpdateFiles.validateApp(app, version: version, bundleID: bundleID)
                return app
            }
            DispatchQueue.main.async {
                guard let self else { try? FileManager.default.removeItem(at: work); return }
                defer {
                    self.installing = false; self.busy = self.checking
                    zct_update_installing(runtime.handle, false)
                    try? FileManager.default.removeItem(at: work)
                }
                var frozen = false
                do {
                    let staged = try result.get()
                    NotificationCenter.default.post(name: .zctWillInstallUpdate, object: nil)
                    try runtime.freeze()
                    frozen = true
                    let backup = try UpdateFiles.replaceApp(staged: staged, installed: installed)
                    let process = Process(); process.executableURL = URL(fileURLWithPath: "/bin/sh")
                    let restart = """
                    while kill -0 "$1" 2>/dev/null; do sleep 1; done
                    /usr/bin/open -g -n "$2" && exit 0
                    /bin/mv "$2" "$3.failed" && /bin/mv "$3" "$2" && /usr/bin/open -g -n "$2"
                    """
                    process.arguments = ["-c", restart, "zoo-type-update", String(ProcessInfo.processInfo.processIdentifier), installed.path, backup.path]
                    do { try process.run() }
                    catch {
                        try FileManager.default.removeItem(at: installed)
                        try FileManager.default.moveItem(at: backup, to: installed)
                        throw error
                    }
                    self.changed("更新完成，輸入法重新啟動；若未切換，請登出再登入")
                    NSApp.terminate(nil)
                } catch {
                    if frozen {
                        do { try runtime.recover() } catch { self.changed("更新失敗，請重新啟動輸入法：" + String(describing: error)); return }
                        NotificationCenter.default.post(name: .zctSettingsChanged, object: nil)
                    }
                    self.changed("更新失敗：" + String(describing: error))
                }
            }
        }
    }
}
