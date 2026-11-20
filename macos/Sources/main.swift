import AppKit
import InputMethodKit
import Carbon

var appRuntime: EngineRuntime?

// 此模式透過真正的 C header / Swift bridge 驗證資料生命週期與輸入結果，不註冊輸入法。
func selfTest(dictionary: String) throws {
    func check(_ condition: Bool, _ message: String) throws {
        if !condition { throw EngineError(description: message) }
    }
    try InputSourceRegistration.selfTest()
    try check(NSClassFromString("ZooTypeInputController") != nil, "Info.plist 的 IMK controller 必須可由 Objective-C runtime 建立")
    try check(Bundle.main.object(forInfoDictionaryKey: "InputMethodServerControllerClass") as? String == "ZooTypeInputController", "IMK class metadata")
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    setenv("ZCT_SELF_TEST", "1", 1)
    setenv("ZCT_HOME", directory.path, 1)
    let runtime = try EngineRuntime(dictionary: dictionary)
    let session = try EngineSession(runtime: runtime, app: "com.apple.TextEdit")
    var time: UInt64 = 1000
    func press(_ code: UInt16, mods: UInt32 = 0) throws -> EngineResult {
        time += 30
        let down = try session.key(code: code, modifiers: mods, time: time)
        time += 30
        _ = try session.key(code: code, modifiers: mods, released: true, time: time)
        return down
    }
    for code in [13, 32, 29, 49, 3, 32, 21] as [UInt16] { _ = try press(code) }
    let chosen = try press(125)
    try check(!chosen.candidatePage.items.isEmpty, "候選窗")
    _ = try press(53)
    let committed = try press(36)
    try check(committed.commit == "天氣", "中文送出：\(committed.commit)")
    try check(try press(0, mods: 8).eaten == false, "Command 快捷鍵必須放行")
    _ = try press(13)
    try check(try session.flush().commit.isEmpty, "未完成注音在失焦時應取消")
    let reconverted = try session.reconvert("天氣")
    try check(!reconverted.composition.text.isEmpty, "重新選字")
    try check(try session.flush().commit == "天氣", "失焦送出")
    runtime.save()
    let preferences = try PreferencesModel(client: runtime)
    // 欄位目錄與設定內容對得上：每個欄位都在某一頁、都讀得到值（欄位數跟著 Rust 目錄走，不寫死）
    let pageIDs = Set(preferences.pages.compactMap { $0["id"] as? String })
    try check(!preferences.fields.isEmpty && !preferences.appFields.isEmpty
        && preferences.fields.allSatisfy { pageIDs.contains($0.page) && ($0.nullable || preferences.value($0.path) != nil) },
        "共用設定欄位完整性")
    try check(preferences.fields.contains { $0.advanced } && preferences.fields.contains { !$0.advanced && $0.page == "general" },
        "一般頁有進階欄位")
    if let select = preferences.fields.first(where: { $0.path == "candidates.select" }) {
        preferences.set("candidates.habit", "windows")
        try check(preferences.lockedValue(select) as? String == "before", "操作習慣鎖定選字對象")
        preferences.set("candidates.habit", "custom")
        try check(preferences.lockedValue(select) == nil, "自訂不鎖定")
    } else {
        try check(false, "設定目錄有選字對象")
    }
    let stale = try PreferencesModel(client: runtime)
    preferences.set("theme.font_size", 18.5)
    try preferences.save()
    try preferences.reload()
    try check((preferences.value("theme.font_size") as? NSNumber)?.doubleValue == 18.5, "設定儲存與重新載入")
    var conflict = false
    do { try stale.save() } catch { conflict = true }
    try check(conflict, "拒絕過期設定覆寫")
    _ = try preferences.manage(["op": "add_phrase", "word": "測試", "bopomofo": "ㄘㄜˋ ㄕˋ"])
    let phrases = try preferences.manage(["op": "list_phrases"])["items"] as? [[String: Any]] ?? []
    try check(phrases.contains { $0["word"] as? String == "測試" }, "共用詞庫管理")
    _ = try preferences.manage(["op": "stats"])
    _ = try preferences.manage(["op": "backup_now"])
    let backups = try preferences.manage(["op": "backups"])["items"] as? [String] ?? []
    try check(!backups.isEmpty, "共用備份管理")
    _ = try preferences.manage(["op": "restore", "day": backups[0]])
    let fresh = try EngineSession(runtime: runtime, app: "com.apple.TextEdit")
    _ = try fresh.key(code: 13, modifiers: 0, time: 2000)
    try check(preferences.supports("sync"), "macOS git 同步能力")
    _ = try preferences.manage(["op": "sync_now"])
    let deadline = Date().addingTimeInterval(15)
    var syncReply = try preferences.manage(["op": "sync_status"])
    while syncReply["running"] as? Bool == true && Date() < deadline {
        Thread.sleep(forTimeInterval: 0.01)
        syncReply = try preferences.manage(["op": "sync_status"])
    }
    try check(syncReply["running"] as? Bool == false && (syncReply["message"] as? String ?? "").contains("成功"), "背景 git 同步與狀態回傳")
    try compositionCaretSelfTest()
    try wezTermInlineAcceptance(runtime: runtime)
    print("macOS Swift / Rust bridge self-test passed")
}

let dictionary = Bundle.main.resourceURL!.appendingPathComponent("dict").path
let settingsNotification = Notification.Name((Bundle.main.bundleIdentifier ?? "ZooType") + ".showSettings")
if CommandLine.arguments.contains("--settings"),
   NSRunningApplication.runningApplications(withBundleIdentifier: Bundle.main.bundleIdentifier ?? "").contains(where: {
       $0.processIdentifier != ProcessInfo.processInfo.processIdentifier
   }) {
    DistributedNotificationCenter.default().postNotificationName(settingsNotification, object: nil, userInfo: nil, deliverImmediately: true)
    exit(0)
}
if CommandLine.arguments.contains("--self-test") {
    do { try selfTest(dictionary: dictionary); exit(0) }
    catch { fputs("ZooType self-test: \(error)\n", stderr); exit(1) }
}
if let index = CommandLine.arguments.firstIndex(of: "--ui-self-test"), CommandLine.arguments.indices.contains(index + 1) {
    do {
        try uiSelfTest(dictionary: dictionary, output: URL(fileURLWithPath: CommandLine.arguments[index + 1]))
        exit(0)
    } catch { fputs("ZooType UI self-test: \(error)\n", stderr); exit(1) }
}
if CommandLine.arguments.contains("--register") {
    do {
        try InputSourceRegistration.register(bundle: .main)
        print("已註冊動物園輸入法。若輸入法選單尚未更新，請重新開啟選單。")
        exit(0)
    } catch { fputs("ZooType registration: \(error)\n", stderr); exit(1) }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var server: IMKServer?
    private var saveTimer: Timer?
    private var settingsWindow: SettingsWindowController?
    func applicationDidFinishLaunching(_ notification: Notification) {
        do { appRuntime = try EngineRuntime(dictionary: dictionary) }
        catch { NSLog("ZooType startup: %@", String(describing: error)); NSApp.terminate(nil); return }
        guard let bundleID = Bundle.main.bundleIdentifier,
              let connection = Bundle.main.object(forInfoDictionaryKey: "InputMethodConnectionName") as? String else {
            NSApp.terminate(nil); return
        }
        server = IMKServer(name: connection, bundleIdentifier: bundleID)
        NSApp.setActivationPolicy(.accessory)
        NotificationCenter.default.addObserver(self, selector: #selector(openSettings(_:)), name: .zctOpenSettings, object: nil)
        DistributedNotificationCenter.default().addObserver(self, selector: #selector(openSettings(_:)), name: settingsNotification, object: nil)
        let mainMenu = NSMenu()
        let appMenu = NSMenu()
        let preferences = appMenu.addItem(withTitle: "動物園輸入法設定…", action: #selector(openSettings(_:)), keyEquivalent: ",")
        preferences.target = self
        let appItem = mainMenu.addItem(withTitle: "動物園輸入法", action: nil, keyEquivalent: "")
        appItem.submenu = appMenu
        let editMenu = NSMenu(title: "編輯")
        for (title, selector, key) in [("剪下", #selector(NSText.cut(_:)), "x"), ("拷貝", #selector(NSText.copy(_:)), "c"),
                                       ("貼上", #selector(NSText.paste(_:)), "v"), ("全選", #selector(NSText.selectAll(_:)), "a")] {
            editMenu.addItem(withTitle: title, action: selector, keyEquivalent: key)
        }
        mainMenu.addItem(withTitle: "編輯", action: nil, keyEquivalent: "").submenu = editMenu
        NSApp.mainMenu = mainMenu
        if let runtime = appRuntime { UpdateManager.shared.start(runtime: runtime) }
        saveTimer = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { _ in appRuntime?.save() }
        if CommandLine.arguments.contains("--settings") { openSettings(nil) }
    }
    @objc func openSettings(_ sender: Any?) {
        guard let runtime = appRuntime else { return }
        do {
            if settingsWindow == nil { settingsWindow = try SettingsWindowController(runtime: runtime) }
            settingsWindow?.open()
        } catch {
            let alert = NSAlert(); alert.messageText = "無法開啟設定"; alert.informativeText = String(describing: error)
            alert.runModal()
        }
    }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        openSettings(nil); return true
    }
    func applicationWillTerminate(_ notification: Notification) { appRuntime?.save() }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.run()
