import Foundation

// The native UI knows this contract, not C pointers or TOML serialization.
protocol PreferencesClient: AnyObject {
    var configURL: URL { get }
    func preferences(_ request: [String: Any]) throws -> [String: Any]
    func relocatePreferences(to path: String) throws -> [String: Any]
}

struct PreferenceField {
    let path: String
    let label: String
    let page: String
    let kind: String
    let choices: [[String: Any]]
    let defaultValue: Any
    let nullable: Bool
    let integer: Bool
    let help: String
    let feature: String?
    /// 另一個欄位（操作習慣）選了特定值時，這個欄位固定成對應的值、不顯示
    let lockBy: String?
    let lockValues: [String: Any]
    /// 少用的設定：收在頁面最後預設收合的「進階」
    let advanced: Bool
    /// 頁面內的分區標題；和前一個欄位不同時顯示
    let section: String
    /// 數字欄位的範圍（儲存時共用層也會檢查）
    let range: (min: Int, max: Int)?
    /// 設定畫面不顯示（只能改設定檔）
    let hidden: Bool
    /// 選了某個選項時順便寫入的其他欄位：選項值 → {欄位路徑: 值}
    let also: [String: [String: Any]]
    init(_ value: [String: Any]) {
        hidden = value["hidden"] as? Bool ?? false
        also = value["also"] as? [String: [String: Any]] ?? [:]
        advanced = value["advanced"] as? Bool ?? false
        section = value["section"] as? String ?? ""
        if let r = value["range"] as? [String: Any], let min = r["min"] as? Int, let max = r["max"] as? Int {
            range = (min, max)
        } else { range = nil }
        let lock = value["lock"] as? [String: Any]
        lockBy = lock?["by"] as? String
        lockValues = lock?["values"] as? [String: Any] ?? [:]
        path = value["path"] as? String ?? ""
        label = value["label"] as? String ?? path
        page = value["page"] as? String ?? ""
        kind = value["kind"] as? String ?? "text"
        choices = value["choices"] as? [[String: Any]] ?? []
        defaultValue = value["default"] ?? NSNull()
        nullable = value["nullable"] as? Bool ?? false
        integer = value["integer"] as? Bool ?? false
        help = value["help"] as? String ?? ""
        feature = value["feature"] as? String
    }
}

final class PreferencesModel {
    let client: PreferencesClient
    let catalog: [String: Any]
    let fields: [PreferenceField]
    let appFields: [PreferenceField]
    var snapshot: [String: Any]
    var document: [String: Any]
    init(client: PreferencesClient) throws {
        self.client = client
        catalog = try client.preferences(["op": "catalog"])["catalog"] as! [String: Any]
        fields = (catalog["fields"] as! [[String: Any]]).map(PreferenceField.init)
        appFields = (catalog["app_fields"] as! [[String: Any]]).map(PreferenceField.init)
        snapshot = try client.preferences(["op": "load"])["snapshot"] as! [String: Any]
        document = snapshot["document"] as! [String: Any]
    }
    var config: [String: Any] { document["config"] as? [String: Any] ?? [:] }
    var apps: [[String: Any]] {
        get { (document["apps"] as? [String: Any])?["app"] as? [[String: Any]] ?? [] }
        set { document["apps"] = ["app": newValue] }
    }
    var pages: [[String: Any]] { catalog["pages"] as? [[String: Any]] ?? [] }
    func supports(_ feature: String) -> Bool {
        (catalog["capabilities"] as? [String: Any])?[feature] as? Bool ?? false
    }
    func value(_ path: String) -> Any? {
        let keys = path.split(separator: ".").map(String.init)
        return (config[keys[0]] as? [String: Any])?[keys[1]]
    }
    /// 依目前的設定，欄位被鎖定成的值；沒有鎖定是 nil（自己設定的值保留在 config 裡）
    func lockedValue(_ field: PreferenceField) -> Any? {
        guard let by = field.lockBy, let key = value(by) as? String else { return nil }
        return field.lockValues[key]
    }
    /// 目前的設定對應到哪一個選項（含順便寫入的欄位）；手動改過設定檔、都不符合時是 nil（顯示「自訂」）
    func chosen(_ field: PreferenceField) -> Int? {
        func same(_ a: Any?, _ b: Any?) -> Bool { (a as? NSObject)?.isEqual(b) == true }
        return field.choices.firstIndex { choice in
            guard same(choice["value"], value(field.path)) else { return false }
            guard let key = choice["value"] as? String, let more = field.also[key] else { return true }
            return more.allSatisfy { same($0.value, value($0.key)) }
        }
    }
    /// 選了第 index 個選項：連同順便寫入的欄位；「自訂」（超出範圍）不動
    func choose(_ field: PreferenceField, _ index: Int) {
        guard field.choices.indices.contains(index), let selected = field.choices[index]["value"] else { return }
        set(field.path, selected)
        if let key = selected as? String { for (path, value) in field.also[key] ?? [:] { set(path, value) } }
    }
    func set(_ path: String, _ value: Any) {
        let keys = path.split(separator: ".").map(String.init)
        var config = self.config
        var section = config[keys[0]] as? [String: Any] ?? [:]
        section[keys[1]] = value; config[keys[0]] = section
        document["config"] = config
    }
    func save() throws {
        let response = try client.preferences(["op": "save", "document": document, "revision": snapshot["revision"]!])
        snapshot = response["snapshot"] as! [String: Any]
        document = snapshot["document"] as! [String: Any]
        NotificationCenter.default.post(name: .zctSettingsChanged, object: nil)
    }
    func reload() throws {
        snapshot = try client.preferences(["op": "load"])["snapshot"] as! [String: Any]
        document = snapshot["document"] as! [String: Any]
    }
    func manage(_ action: [String: Any]) throws -> [String: Any] {
        let response = try client.preferences(["op": "manage", "action": action])
        return response["reply"] as! [String: Any]
    }
}
