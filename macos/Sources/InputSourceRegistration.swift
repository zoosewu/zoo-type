import Foundation
import Carbon

// TISEnableInputSource can return noErr without updating the enabled-source list.
// Verify persisted state and only append this bundle's missing entries.
enum InputSourceRegistration {
    private static let domain = "com.apple.HIToolbox" as CFString
    private static let key = "AppleEnabledInputSources" as CFString

    static func requiredEntries(bundleID: String, modes: [String]) -> [[String: Any]] {
        var entries: [[String: Any]] = [["Bundle ID": bundleID, "InputSourceKind": "Keyboard Input Method"]]
        entries += modes.map { ["Bundle ID": bundleID, "InputSourceKind": "Input Mode", "Input Mode": $0] }
        return entries
    }
    static func contains(_ entries: [[String: Any]], _ required: [String: Any]) -> Bool {
        entries.contains { entry in required.allSatisfy { key, value in entry[key] as? String == value as? String } }
    }
    static func merging(_ entries: [[String: Any]], required: [[String: Any]]) -> [[String: Any]] {
        var updated = entries
        for entry in required where !contains(updated, entry) { updated.append(entry) }
        return updated
    }
    private static func defaults(_ arguments: [String]) throws -> Data {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/defaults")
        task.arguments = arguments
        let output = Pipe(); let error = Pipe()
        task.standardOutput = output; task.standardError = error
        try task.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        let diagnostic = error.fileHandleForReading.readDataToEndOfFile()
        task.waitUntilExit()
        guard task.terminationStatus == 0 else {
            throw EngineError(description: "macOS 偏好設定服務拒絕存取：" + String(decoding: diagnostic, as: UTF8.self))
        }
        return data
    }
    private static func enabledPreferences() throws -> [[String: Any]] {
        let data = try defaults(["export", domain as String, "-"])
        let plist = try PropertyListSerialization.propertyList(from: data, options: [], format: nil)
        guard let dictionary = plist as? [String: Any], let entries = dictionary[key as String] as? [[String: Any]], !entries.isEmpty else {
            throw EngineError(description: "無法讀取既有的 macOS 輸入來源清單，未修改任何設定")
        }
        return entries
    }
    private static func sources(bundleID: String, includeDisabled: Bool = true) -> [TISInputSource] {
        guard let list = TISCreateInputSourceList([kTISPropertyBundleID as String: bundleID] as CFDictionary, includeDisabled) else { return [] }
        return list.takeRetainedValue() as! [TISInputSource]
    }
    private static func string(_ source: TISInputSource, _ key: CFString) -> String? {
        guard let p = TISGetInputSourceProperty(source, key) else { return nil }
        return Unmanaged<CFString>.fromOpaque(p).takeUnretainedValue() as String
    }
    private static func flag(_ source: TISInputSource, _ key: CFString) -> Bool {
        guard let p = TISGetInputSourceProperty(source, key) else { return false }
        return CFBooleanGetValue(Unmanaged<CFBoolean>.fromOpaque(p).takeUnretainedValue())
    }
    static func register(bundle: Bundle) throws {
        // 輸入來源清單是每個使用者自己的：root 讀寫的是 root 的清單，登記不到目前登入的帳號
        guard getuid() != 0 else { throw EngineError(description: "請不要用 sudo 安裝；直接以目前登入的帳號執行") }
        guard let bundleID = bundle.bundleIdentifier else { throw EngineError(description: "輸入法缺少 Bundle ID") }
        let status = TISRegisterInputSource(bundle.bundleURL as CFURL)
        guard status == noErr else { throw EngineError(description: "TISRegisterInputSource: \(status)") }
        let modeDictionary = bundle.object(forInfoDictionaryKey: "ComponentInputModeDict") as? [String: Any]
        let modes = modeDictionary?["tsVisibleInputModeOrderedArrayKey"] as? [String] ?? []
        guard modes.allSatisfy({ $0.hasPrefix(bundleID + ".") }) else { throw EngineError(description: "輸入模式識別不屬於此輸入法") }
        let all = sources(bundleID: bundleID)
        guard !all.isEmpty else {
            throw EngineError(description: "檔案已安裝，但目前登入階段尚未登記此輸入法；請登出再登入")
        }
        // Enable the parent before the modes regardless of enumeration order.
        let ordered = all.sorted {
            (string($0, kTISPropertyInputSourceID) == bundleID ? 0 : 1) <
            (string($1, kTISPropertyInputSourceID) == bundleID ? 0 : 1)
        }
        for source in ordered where flag(source, kTISPropertyInputSourceIsEnableCapable) {
            let result = TISEnableInputSource(source)
            guard result == noErr else { throw EngineError(description: "TISEnableInputSource: \(result)") }
        }
        let required = requiredEntries(bundleID: bundleID, modes: modes)
        let previous = try enabledPreferences()
        let updated = merging(previous, required: required)
        if updated.count != previous.count {
            let applicationSupport = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            let directory = applicationSupport.appendingPathComponent(bundleID.hasSuffix(".dev") ? "ZooType-dev" : "ZooType")
                .appendingPathComponent("local/registration-backups")
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let backup = directory.appendingPathComponent(UUID().uuidString + ".plist")
            try PropertyListSerialization.data(fromPropertyList: previous, format: .xml, options: 0).write(to: backup, options: .atomic)
            let missing = required.filter { !contains(previous, $0) }
            let values = try missing.map { entry -> String in
                let data = try PropertyListSerialization.data(fromPropertyList: entry, format: .xml, options: 0)
                let xml = String(decoding: data, as: UTF8.self)
                guard let start = xml.range(of: "<dict>"), let end = xml.range(of: "</dict>", options: .backwards) else {
                    throw EngineError(description: "無法編碼輸入來源紀錄")
                }
                return String(xml[start.lowerBound..<end.upperBound])
            }
            _ = try defaults(["write", domain as String, key as String, "-array-add"] + values)
            print("已補齊父輸入法與注音模式的啟用紀錄；原清單備份：\(backup.path)")
            DistributedNotificationCenter.default().postNotificationName(
                Notification.Name(kTISNotifyEnabledKeyboardInputSourcesChanged as String), object: nil, userInfo: nil, deliverImmediately: true)
        }
        let persisted = try enabledPreferences()
        guard required.allSatisfy({ contains(persisted, $0) }) else {
            throw EngineError(description: "macOS 未保留輸入來源啟用紀錄，註冊尚未完成")
        }
        print("已驗證父輸入法與 \(modes.count) 個模式都在 macOS 啟用清單。")
    }
    static func selfTest() throws {
        let original: [[String: Any]] = [
            ["InputSourceKind": "Keyboard Layout", "KeyboardLayout Name": "ABC", "KeyboardLayout ID": 252],
            ["InputSourceKind": "Keyboard Input Method", "Bundle ID": "com.apple.inputmethod.TCIM"]
        ]
        let required = requiredEntries(bundleID: "com.example.inputmethod.Test", modes: ["com.example.inputmethod.Test.Zhuyin"])
        let updated = merging(original, required: required)
        guard updated.count == original.count + 2,
              NSArray(array: Array(updated.prefix(original.count))).isEqual(to: original),
              NSArray(array: merging(updated, required: required)).isEqual(to: updated),
              merging(original + [required[0]], required: required).count == original.count + 2 else {
            throw EngineError(description: "輸入來源清單合併不可移除其他鍵盤或產生重複項目")
        }
    }
}
