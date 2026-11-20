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
    static func newer(_ version: String, than current: String) -> Bool {
        version.withCString { v in current.withCString { zct_version_newer(v, $0) } }
    }
    static func verify(archive: URL, signature: URL, version: String) throws {
        guard archive.path.withCString({ file in signature.path.withCString { sig in version.withCString { zct_update_verify(file, sig, $0) } } }) else { throw EngineError.current }
    }
    @discardableResult
    static func run(_ executable: String, _ arguments: [String]) throws -> String {
        let process = Process(); process.executableURL = URL(fileURLWithPath: executable); process.arguments = arguments
        let output = Pipe(); process.standardOutput = output; process.standardError = output
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile(); process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw EngineError(description: "更新操作失敗：" + String(decoding: data, as: UTF8.self).prefix(1500)) }
        return String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    }
    static func download(_ url: URL, to destination: URL, maximum: Int) throws {
        guard url.scheme == "https" else { throw EngineError(description: "更新只接受 HTTPS 下載") }
        try run("/usr/bin/curl", ["-q", "--fail", "--silent", "--show-error", "--location", "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "15", "--max-time", "300", "--max-filesize", String(maximum), "--output", destination.path, url.absoluteString])
        let size = (try destination.resourceValues(forKeys: [.fileSizeKey])).fileSize ?? 0
        guard size > 0 && size <= maximum else { throw EngineError(description: "更新檔案大小不符") }
    }
    static func assetURL(_ url: String, tag: String, name: String) throws -> URL {
        guard let value = URL(string: url), value.scheme == "https", value.host == "github.com",
              value.user == nil, value.password == nil, value.port == nil, value.query == nil, value.fragment == nil,
              value.path == "/zoosewu/zoo-type/releases/download/\(tag)/\(name)" else { throw EngineError(description: "更新來源不符官方 Release") }
        return value
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

// All state belongs to the main thread. Network and extraction use background queues.
final class UpdateManager {
    static let shared = UpdateManager()
    private(set) var message = "尚未檢查更新"
    private(set) var available: String?
    private(set) var busy = false
    private weak var runtime: EngineRuntime?
    private var lastAutomatic: Date?
    private var timer: Timer?
    private let queue = DispatchQueue(label: "ZooType.updates", qos: .utility)
    private var current: String { Bundle.main.object(forInfoDictionaryKey: "ZooTypeVersion") as? String ?? "0.0.0" }
    var supported: Bool {
        Bundle.main.bundleIdentifier == "com.zoosewu.inputmethod.ZooType" && ProcessInfo.processInfo.environment["ZCT_HOME"] == nil
    }
    private var directory: URL? { runtime?.configURL.deletingLastPathComponent().appendingPathComponent("local/update") }
    private func archive(_ directory: URL, _ version: String) -> URL { directory.appendingPathComponent("zoo-type-\(version)-macos-\(UpdateFiles.architecture).zip") }
    func start(runtime: EngineRuntime) {
        self.runtime = runtime
        guard supported else { message = "開發版不提供自動更新"; return }
        if let directory {
            busy = true
            let current = self.current
            queue.async {
                let suffix = "-macos-" + UpdateFiles.architecture + ".zip"
                let prefix = "zoo-type-"
                let files = (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
                var latest: String?
                for file in files {
                    let name = file.lastPathComponent
                    guard name.hasPrefix(prefix), name.hasSuffix(suffix) else { continue }
                    let version = String(name.dropFirst(prefix.count).dropLast(suffix.count))
                    guard UpdateFiles.newer(version, than: latest ?? current) else { continue }
                    do {
                        try UpdateFiles.verify(archive: file, signature: file.appendingPathExtension("sig"), version: version)
                        latest = version
                    } catch { continue }
                }
                let verified = latest
                DispatchQueue.main.async {
                    self.busy = false
                    if let verified { self.available = verified; self.changed("更新已驗證：v" + verified) }
                }
            }
        }
        timer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in self?.automatic() }
        // Same cadence as Windows: first check after five minutes, then daily.
        lastAutomatic = Date().addingTimeInterval(-24 * 3600 + 300)
    }
    private func changed(_ message: String) {
        self.message = message
        NotificationCenter.default.post(name: .zctUpdateChanged, object: nil)
    }
    private func automatic() {
        guard let runtime, !busy, Date().timeIntervalSince(lastAutomatic ?? .distantPast) >= 24 * 3600,
              let config = try? runtime.settings(), (config["update"] as? [String: Any])?["auto_check"] as? Bool == true else { return }
        lastAutomatic = Date(); check()
    }
    func check() {
        guard supported, !busy, let directory else { return }
        busy = true; changed("檢查與下載更新中…")
        let current = self.current
        queue.async { [weak self] in
            let result = Result { () throws -> String? in
                let manager = FileManager.default
                try manager.createDirectory(at: directory, withIntermediateDirectories: true)
                let work = directory.appendingPathComponent(UUID().uuidString)
                try manager.createDirectory(at: work, withIntermediateDirectories: true)
                defer { try? manager.removeItem(at: work) }
                let metadata = work.appendingPathComponent("release.json")
                try UpdateFiles.download(URL(string: "https://api.github.com/repos/zoosewu/zoo-type/releases/latest")!, to: metadata, maximum: 2 * 1024 * 1024)
                guard let release = try JSONSerialization.jsonObject(with: Data(contentsOf: metadata)) as? [String: Any],
                      let tag = release["tag_name"] as? String, release["draft"] as? Bool != true, release["prerelease"] as? Bool != true else { throw EngineError(description: "更新資訊格式錯誤") }
                let version = String(tag.dropFirst(tag.hasPrefix("v") ? 1 : 0))
                guard UpdateFiles.newer(version, than: current) else { return nil }
                let name = "zoo-type-\(version)-macos-\(UpdateFiles.architecture).zip"
                let assets = release["assets"] as? [[String: Any]] ?? []
                func url(_ asset: String) throws -> URL {
                    guard let value = assets.first(where: { $0["name"] as? String == asset })?["browser_download_url"] as? String else { throw EngineError(description: "Release 缺少 " + asset) }
                    return try UpdateFiles.assetURL(value, tag: tag, name: asset)
                }
                let file = work.appendingPathComponent(name), signature = file.appendingPathExtension("sig")
                try UpdateFiles.download(url(name), to: file, maximum: 256 * 1024 * 1024)
                try UpdateFiles.download(url(name + ".sig"), to: signature, maximum: 4096)
                try UpdateFiles.verify(archive: file, signature: signature, version: version)
                for source in [file, signature] {
                    let destination = directory.appendingPathComponent(source.lastPathComponent)
                    if manager.fileExists(atPath: destination.path) { try manager.removeItem(at: destination) }
                    try manager.moveItem(at: source, to: destination)
                }
                return version
            }
            DispatchQueue.main.async {
                guard let self else { return }; self.busy = false
                switch result {
                case .success(let version): self.available = version; self.changed(version.map { "更新已驗證：v" + $0 } ?? "目前已是最新版")
                case .failure(let error): self.changed("檢查更新失敗：" + String(describing: error))
                }
            }
        }
    }
    func install() {
        guard supported, !busy, let version = available, let directory, let runtime else { return }
        let installed = Bundle.main.bundleURL.standardizedFileURL
        let expected = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Input Methods/動物園輸入法.app").standardizedFileURL
        guard installed == expected else { changed("請先將正式版安裝至 Library/Input Methods，再使用自動更新"); return }
        busy = true; changed("驗證並準備安裝…")
        let file = archive(directory, version), bundleID = Bundle.main.bundleIdentifier!
        queue.async { [weak self] in
            let work = directory.appendingPathComponent("staged-" + UUID().uuidString)
            let result = Result { () throws -> URL in
                try UpdateFiles.verify(archive: file, signature: file.appendingPathExtension("sig"), version: version)
                try FileManager.default.createDirectory(at: work, withIntermediateDirectories: true)
                try UpdateFiles.run("/usr/bin/ditto", ["-x", "-k", file.path, work.path])
                let app = work.appendingPathComponent("zoo-type-\(version)-macos-\(UpdateFiles.architecture)/動物園輸入法.app")
                try UpdateFiles.validateApp(app, version: version, bundleID: bundleID)
                return app
            }
            DispatchQueue.main.async {
                guard let self else { try? FileManager.default.removeItem(at: work); return }
                defer { self.busy = false; try? FileManager.default.removeItem(at: work) }
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
