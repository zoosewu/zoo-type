import Foundation
import Security

// Secrets stay in the login Keychain; only non-secret AI options enter config/git.
enum Credentials {
    private static let testService = "ZooType.self-test." + UUID().uuidString
    static var service: String {
        if ProcessInfo.processInfo.environment["ZCT_SELF_TEST"] == "1" { return testService }
        return Bundle.main.bundleIdentifier ?? "com.zoosewu.inputmethod.ZooType"
    }
    private static func query(_ service: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service, kSecAttrAccount as String: "ai-api-key"]
    }
    private static func failure(_ status: OSStatus) -> EngineError {
        EngineError(description: "Keychain：" + (SecCopyErrorMessageString(status, nil) as String? ?? String(status)))
    }
    static func read(service: String = service) throws -> String? {
        var request = query(service); request[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data, let key = String(data: data, encoding: .utf8) else { throw failure(status) }
        return key
    }
    static func write(_ value: String, service: String = service) throws {
        let request = query(service)
        if value.isEmpty {
            let status = SecItemDelete(request as CFDictionary)
            guard status == errSecSuccess || status == errSecItemNotFound else { throw failure(status) }
            return
        }
        let data = Data(value.utf8)
        let status = SecItemUpdate(request as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var item = request; item[kSecValueData as String] = data
            let added = SecItemAdd(item as CFDictionary, nil)
            guard added == errSecSuccess else { throw failure(added) }
        } else if status != errSecSuccess { throw failure(status) }
    }
}

enum AIConnection {
    // Does not touch runtime handles: callers run this on a background queue.
    static func test(config: [String: Any], key: String) throws -> String {
        let json = String(decoding: try JSONSerialization.data(withJSONObject: config), as: UTF8.self)
        let result = json.withCString { options in key.withCString { zct_ai_test(options, $0) } }
        guard let result else { throw EngineError.current }
        defer { zct_string_free(result) }
        return String(cString: result)
    }
}
