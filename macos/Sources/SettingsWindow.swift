import AppKit

private final class SettingsBackground: NSView {
    override var isOpaque: Bool { true }
    override func draw(_ dirtyRect: NSRect) { NSColor.windowBackgroundColor.setFill(); bounds.fill() }
}
private final class FlippedStackView: NSStackView { override var isFlipped: Bool { true } }

final class SettingsWindowController: NSWindowController, NSSearchFieldDelegate, NSTableViewDataSource, NSTableViewDelegate {
    private let model: PreferencesModel
    private var bindings: [() -> Void] = []
    private var jsonDrafts: [String: String] = [:]
    private var page = 0
    private var showAdvanced = false
    private var selectedApp = 0
    private var nav: [NSButton] = []
    private let body = FlippedStackView()
    private let status = NSTextField(labelWithString: "")
    private let preview = CandidateView()
    private var collecting = false
    private var phrases: [[String: Any]] = []
    private var filtered: [[String: Any]] = []
    private var phraseTable: NSTableView?
    private var phraseSearch = ""
    private var wordField: NSTextField?
    private var readingField: NSTextField?
    private var syncTimer: Timer?
    private weak var updateStatusLabel: NSTextField?
    private weak var updateInstallButton: NSButton?
    private weak var syncStatusLabel: NSTextField?
    private var backupPicker: NSPopUpButton?

    init(runtime: PreferencesClient) throws {
        model = try PreferencesModel(client: runtime)
        let title = (Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "動物園輸入法") + " 設定"
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 860, height: 640),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = title
        window.contentView = SettingsBackground(frame: NSRect(x: 0, y: 0, width: 860, height: 640))
        window.minSize = NSSize(width: 820, height: 600)
        window.isReleasedWhenClosed = false
        super.init(window: window)
        buildWindow(); window.center()
        syncTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in self?.refreshSyncStatus(); self?.refreshUpdateStatus() }
    }
    deinit { syncTimer?.invalidate() }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func previewPage(_ index: Int) { collect(); page = index; showPage() }
    func open() { showWindow(nil); NSApp.activate(ignoringOtherApps: true); window?.makeKeyAndOrderFront(nil) }
    private func buildWindow() {
        guard let root = window?.contentView else { return }
        let sidebar = NSStackView()
        sidebar.orientation = .vertical; sidebar.alignment = .leading; sidebar.spacing = 5
        sidebar.edgeInsets = NSEdgeInsets(top: 20, left: 12, bottom: 16, right: 12)
        let title = NSTextField(labelWithString: "動物園輸入法"); title.font = .boldSystemFont(ofSize: 15)
        sidebar.addArrangedSubview(title)
        let subtitle = NSTextField(labelWithString: "設定"); subtitle.textColor = .secondaryLabelColor
        sidebar.addArrangedSubview(subtitle)
        for (i, descriptor) in model.pages.enumerated() {
            let button = NSButton(title: descriptor["title"] as? String ?? "", target: self, action: #selector(selectPage(_:)))
            button.bezelStyle = .regularSquare; button.setButtonType(.pushOnPushOff); button.alignment = .left; button.tag = i
            button.widthAnchor.constraint(equalToConstant: 112).isActive = true
            button.heightAnchor.constraint(equalToConstant: 32).isActive = true
            sidebar.addArrangedSubview(button); nav.append(button)
        }
        sidebar.addArrangedSubview(NSView())
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.drawsBackground = false
        body.orientation = .vertical; body.alignment = .leading; body.spacing = 12
        body.edgeInsets = NSEdgeInsets(top: 24, left: 24, bottom: 24, right: 24)
        body.translatesAutoresizingMaskIntoConstraints = false; scroll.documentView = body
        body.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor).isActive = true
        let right = NSView()
        let save = button("儲存設定", #selector(saveSettings(_:)))
        save.keyEquivalent = "s"; save.keyEquivalentModifierMask = .command
        let reload = button("重新載入", #selector(reloadSettings(_:)))
        let folder = button("開啟資料夾", #selector(openFolder(_:)))
        status.font = .systemFont(ofSize: 11); status.textColor = .secondaryLabelColor
        for view in [sidebar, right] { view.translatesAutoresizingMaskIntoConstraints = false; root.addSubview(view) }
        for view in [scroll, save, reload, folder, status] { view.translatesAutoresizingMaskIntoConstraints = false; right.addSubview(view) }
        NSLayoutConstraint.activate([
            sidebar.leadingAnchor.constraint(equalTo: root.leadingAnchor), sidebar.topAnchor.constraint(equalTo: root.topAnchor),
            sidebar.bottomAnchor.constraint(equalTo: root.bottomAnchor), sidebar.widthAnchor.constraint(equalToConstant: 140),
            right.leadingAnchor.constraint(equalTo: sidebar.trailingAnchor), right.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            right.topAnchor.constraint(equalTo: root.topAnchor), right.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: right.leadingAnchor), scroll.trailingAnchor.constraint(equalTo: right.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: right.topAnchor), scroll.bottomAnchor.constraint(equalTo: right.bottomAnchor, constant: -64),
            save.trailingAnchor.constraint(equalTo: right.trailingAnchor, constant: -20), save.bottomAnchor.constraint(equalTo: right.bottomAnchor, constant: -18),
            reload.trailingAnchor.constraint(equalTo: save.leadingAnchor, constant: -10), reload.centerYAnchor.constraint(equalTo: save.centerYAnchor),
            folder.leadingAnchor.constraint(equalTo: right.leadingAnchor, constant: 24), folder.centerYAnchor.constraint(equalTo: save.centerYAnchor),
            status.leadingAnchor.constraint(equalTo: folder.trailingAnchor, constant: 12), status.centerYAnchor.constraint(equalTo: save.centerYAnchor),
            status.trailingAnchor.constraint(lessThanOrEqualTo: reload.leadingAnchor, constant: -12)
        ])
        showPage()
    }
    private func button(_ title: String, _ selector: Selector) -> NSButton {
        let result = NSButton(title: title, target: self, action: selector); result.bezelStyle = .rounded; return result
    }
    private func header(_ text: String) {
        let label = NSTextField(labelWithString: text); label.font = .boldSystemFont(ofSize: 16)
        body.addArrangedSubview(label)
    }
    private func note(_ text: String) {
        let label = NSTextField(wrappingLabelWithString: text)
        label.textColor = .secondaryLabelColor; label.font = .systemFont(ofSize: 12); label.preferredMaxLayoutWidth = 610
        body.addArrangedSubview(label)
    }
    private func collect() {
        guard !collecting else { return }; collecting = true
        for binding in bindings { binding() }
        collecting = false
    }
    private func parseJSONDrafts() throws {
        for (path, text) in jsonDrafts {
            do { model.set(path, try JSONSerialization.jsonObject(with: Data(text.utf8), options: [.fragmentsAllowed])) }
            catch { throw EngineError(description: "\(path) 需要有效的 JSON：\(error)") }
        }
    }
    @objc private func changed(_ sender: Any?) {
        collect(); status.stringValue = "尚未儲存"
        let id = model.pages[page]["id"] as? String
        if id == "appearance" { updatePreview() }
        // 換了選項（例如操作習慣）：重畫這一頁，顯示或隱藏被鎖定的欄位（文字欄位打字時不重畫，免得失去焦點）
        if sender is NSPopUpButton, model.fields.contains(where: { $0.page == id && $0.lockBy != nil }) {
            showPage(); status.stringValue = "尚未儲存"
        }
    }
    func controlTextDidChange(_ notification: Notification) {
        if (notification.object as? NSControl)?.identifier?.rawValue == "phrase-search" {
            phraseSearch = (notification.object as? NSTextField)?.stringValue ?? ""; filterPhrases()
        } else { changed(notification.object) }
    }
    private func row(_ title: String, _ control: NSView) {
        let label = NSTextField(labelWithString: title); label.widthAnchor.constraint(equalToConstant: 160).isActive = true
        let row = NSStackView(views: [label, control]); row.spacing = 12; row.alignment = .centerY
        body.addArrangedSubview(row)
    }
    private func render(_ field: PreferenceField, appIndex: Int? = nil) {
        // 被操作習慣鎖定：不顯示，也不寫回（自己的設定保留著）
        if appIndex == nil && model.lockedValue(field) != nil { return }
        let current = appIndex.map { model.apps[$0][field.path] ?? NSNull() } ?? model.value(field.path) ?? field.defaultValue
        let set: (Any) -> Void = { [weak self] value in
            guard let self else { return }
            if let index = appIndex {
                var apps = self.model.apps
                guard apps.indices.contains(index) else { return }
                apps[index][field.path] = value; self.model.apps = apps
            } else { self.model.set(field.path, value) }
        }
        let enabled = field.feature.map(model.supports) ?? true
        let title = field.label
        switch field.kind {
        case "bool":
            let control = NSButton(checkboxWithTitle: title, target: self, action: #selector(changed(_:)))
            control.state = current as? Bool == true ? .on : .off; control.isEnabled = enabled
            bindings.append { [weak control] in set(control?.state == .on) }; body.addArrangedSubview(control)
        case "choice":
            let control = NSPopUpButton(); control.addItems(withTitles: field.choices.map { $0["label"] as? String ?? "" })
            // 全域設定：連同順便寫入的欄位比對；手動改過設定檔、都不符合時多一個「自訂」，選著它儲存時不動
            let chosen = appIndex == nil ? model.chosen(field) : field.choices.firstIndex { ($0["value"] as? NSObject)?.isEqual(current) == true }
            if appIndex == nil && chosen == nil { control.addItem(withTitle: "自訂") }
            control.selectItem(at: chosen ?? (appIndex == nil ? field.choices.count : 0))
            control.target = self; control.action = #selector(changed(_:)); control.isEnabled = enabled
            control.widthAnchor.constraint(equalToConstant: 280).isActive = true
            bindings.append { [weak self, weak control] in
                guard let self, let control, field.choices.indices.contains(control.indexOfSelectedItem) else { return }
                if appIndex == nil { self.model.choose(field, control.indexOfSelectedItem) }
                else { set(field.choices[control.indexOfSelectedItem]["value"] ?? NSNull()) }
            }; row(title, control)
        case "color":
            let control = NSColorWell(); control.color = CandidateTheme.color(current as? String) ?? .white
            control.target = self; control.action = #selector(changed(_:)); control.isEnabled = enabled
            bindings.append { [weak control] in
                guard let color = control?.color.usingColorSpace(.deviceRGB) else { return }
                set(String(format: "#%02X%02X%02X", Int((color.redComponent * 255).rounded()), Int((color.greenComponent * 255).rounded()), Int((color.blueComponent * 255).rounded())))
            }; row(title, control)
        case "json":
            let text = NSTextView(); text.isRichText = false; text.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            let data = (try? JSONSerialization.data(withJSONObject: current, options: [.sortedKeys, .prettyPrinted])) ?? Data("{}".utf8)
            text.string = jsonDrafts[field.path] ?? String(decoding: data, as: UTF8.self); text.isEditable = enabled
            let scroll = NSScrollView(); scroll.documentView = text; scroll.hasVerticalScroller = true; scroll.borderType = .bezelBorder
            scroll.widthAnchor.constraint(equalToConstant: 420).isActive = true; scroll.heightAnchor.constraint(equalToConstant: 80).isActive = true
            bindings.append { [weak self, weak text] in self?.jsonDrafts[field.path] = text?.string ?? "{}" }
            row(title, scroll)
        default:
            let value = current is NSNull ? "" : (current as? String ?? String(describing: current))
            let control = NSTextField(string: value); control.delegate = self; control.isEnabled = enabled
            control.widthAnchor.constraint(equalToConstant: 300).isActive = true
            control.target = self; control.action = #selector(changed(_:))
            let integer = field.integer
            if field.kind == "number" {
                let formatter = NumberFormatter(); formatter.allowsFloats = !integer
                if let range = field.range { formatter.minimum = NSNumber(value: range.min); formatter.maximum = NSNumber(value: range.max) }
                control.formatter = formatter
            }
            bindings.append { [weak control] in
                guard let control else { return }
                if field.kind == "number" { set(integer ? (Int(control.stringValue) ?? 0) as Any : (Double(control.stringValue) ?? 0) as Any) }
                else { set(field.nullable && control.stringValue.isEmpty ? NSNull() : control.stringValue as Any) }
            }; row(title, control)
        }
        if !field.help.isEmpty && field.kind != "hotkey" { note(field.help) }
    }
    private func isLocked(_ field: PreferenceField) -> Bool { model.lockedValue(field) != nil }
    @objc private func selectPage(_ sender: NSButton) { collect(); page = sender.tag; showPage() }
    @objc private func toggleAdvanced(_ sender: Any?) { collect(); showAdvanced.toggle(); showPage() }
    private func showPage() {
        bindings = []; phraseTable = nil
        for view in body.arrangedSubviews { body.removeArrangedSubview(view); view.removeFromSuperview() }
        for button in nav { button.state = button.tag == page ? .on : .off }
        let descriptor = model.pages[page]
        let id = descriptor["id"] as? String ?? ""
        let title = NSTextField(labelWithString: descriptor["title"] as? String ?? "")
        title.font = .boldSystemFont(ofSize: 22); body.addArrangedSubview(title)
        note(descriptor["help"] as? String ?? "")
        let fields = model.fields.filter { $0.page == id }
        if fields.contains(where: { $0.feature.map { !model.supports($0) } ?? false }) {
            note("灰色欄位的功能尚未在此平台提供；既有值會保留，與其他平台共用設定時不會被清除。")
        }
        var section = ""
        for field in fields where !field.advanced && !field.hidden {
            // 被操作習慣鎖定的欄位不顯示，也不單獨帶出分區標題
            if isLocked(field) { continue }
            if field.section != section {
                section = field.section
                if !section.isEmpty { header(section) }
            }
            render(field)
        }
        if fields.contains(where: { $0.advanced }) {
            body.addArrangedSubview(button(showAdvanced ? "▾ 進階" : "▸ 進階", #selector(toggleAdvanced(_:))))
            if showAdvanced { for field in fields where field.advanced && !field.hidden { render(field) } }
        }
        switch id {
        case "appearance": body.addArrangedSubview(preview); updatePreview()
        case "apps": appEditor()
        case "dictionary": dictionaryEditor()
        case "learning": learningView()
        case "sync": locationView()
        case "ai":
            let key = button("儲存 API Key…", #selector(saveApiKey(_:))); key.isEnabled = model.supports("api_key"); body.addArrangedSubview(key)
            let test = button("測試連線", #selector(testAi(_:))); test.isEnabled = model.supports("ai"); body.addArrangedSubview(test)
            do { let present = try model.manage(["op": "api_key_status"])["present"] as? Bool == true
                note(present ? "API Key 已存於 macOS Keychain。" : "未設定 API Key；本機免金鑰服務可留空。")
            } catch { note("無法讀取 Keychain：" + String(describing: error)) }
        case "about":
            let version = Bundle.main.object(forInfoDictionaryKey: "ZooTypeVersion") as? String ?? ""
            body.addArrangedSubview(NSTextField(labelWithString: "版本 \(version)"))
            note("設定欄位、預設值、驗證、儲存與管理操作由共用設定層提供。")
            let update = button("檢查更新", #selector(checkUpdate(_:))); update.isEnabled = UpdateManager.shared.supported
            body.addArrangedSubview(update)
            let install = button("安裝更新並重新啟動", #selector(installUpdate(_:))); body.addArrangedSubview(install); updateInstallButton = install
            let label = NSTextField(wrappingLabelWithString: ""); label.preferredMaxLayoutWidth = 520
            body.addArrangedSubview(label); updateStatusLabel = label; refreshUpdateStatus()
            body.addArrangedSubview(button("編輯設定檔…", #selector(editFile(_:))))
        default: break
        }
        if id == "hotkeys" { note("留空表示不綁定；ctrl、shift、alt（Option）、win（Command）加按鍵，例如 shift+space。") }
        body.layoutSubtreeIfNeeded()
        if let scroll = body.enclosingScrollView { scroll.contentView.scroll(to: .zero); scroll.reflectScrolledClipView(scroll.contentView) }
    }
    private func updatePreview() {
        preview.update(CandidateDisplay(items: ["天氣", "天汽", "添氣"], labels: ["1", "2", "3"], highlight: 0, page: 0, totalPages: 3), theme: CandidateTheme(config: model.config))
        preview.invalidateIntrinsicContentSize()
    }
    private func appEditor() {
        let picker = NSPopUpButton()
        picker.addItems(withTitles: model.apps.enumerated().map { index, app in
            let name = app["name"] as? String ?? ""; return name.isEmpty ? "規則 \(index + 1)" : name
        })
        selectedApp = min(selectedApp, max(0, model.apps.count - 1)); picker.selectItem(at: selectedApp)
        picker.target = self; picker.action = #selector(selectApp(_:))
        let actions = NSStackView(views: [picker, button("新增", #selector(addRule(_:))), button("移除", #selector(removeRule(_:)))])
        actions.spacing = 8; body.addArrangedSubview(actions)
        if model.apps.indices.contains(selectedApp) { for field in model.appFields { render(field, appIndex: selectedApp) } }
    }
    @objc private func selectApp(_ sender: NSPopUpButton) { collect(); selectedApp = sender.indexOfSelectedItem; showPage() }
    @objc private func addRule(_ sender: Any?) {
        collect(); var apps = model.apps
        apps.append(Dictionary(uniqueKeysWithValues: model.appFields.map { ($0.path, $0.defaultValue) }))
        apps[apps.count - 1]["name"] = "新規則"; model.apps = apps; selectedApp = apps.count - 1; showPage()
    }
    @objc private func removeRule(_ sender: Any?) {
        collect(); var apps = model.apps
        if apps.indices.contains(selectedApp) { apps.remove(at: selectedApp); model.apps = apps }; showPage()
    }
    private func dictionaryEditor() {
        let search = NSSearchField(string: phraseSearch); search.delegate = self; search.identifier = NSUserInterfaceItemIdentifier("phrase-search")
        search.placeholderString = "搜尋詞語或讀音"; search.widthAnchor.constraint(equalToConstant: 500).isActive = true; body.addArrangedSubview(search)
        let table = NSTableView(); table.delegate = self; table.dataSource = self; table.allowsMultipleSelection = false
        for (id, title, width) in [("word", "詞語", 150.0), ("bopomofo", "注音／縮寫", 245.0), ("state", "狀態", 80.0)] {
            let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier(id)); column.title = title; column.width = width; table.addTableColumn(column)
        }
        let scroll = NSScrollView(); scroll.documentView = table; scroll.hasVerticalScroller = true; scroll.borderType = .bezelBorder
        scroll.widthAnchor.constraint(equalToConstant: 520).isActive = true; scroll.heightAnchor.constraint(equalToConstant: 210).isActive = true
        body.addArrangedSubview(scroll); phraseTable = table
        let word = NSTextField(); word.placeholderString = "詞語／英文單字"; word.widthAnchor.constraint(equalToConstant: 280).isActive = true
        let reading = NSTextField(); reading.placeholderString = "注音或英文縮寫（英文單字可留空）"; reading.widthAnchor.constraint(equalToConstant: 300).isActive = true
        row("詞語", word); row("讀音", reading); wordField = word; readingField = reading
        let editing = NSStackView(views: [button("加入", #selector(addPhrase(_:))), button("更新", #selector(editPhrase(_:))), button("查讀音", #selector(suggestReading(_:)))])
        editing.spacing = 8; body.addArrangedSubview(editing)
        let actions = NSStackView(views: [button("設為優先", #selector(pinPhrase(_:))), button("封鎖", #selector(banPhrase(_:))), button("恢復一般", #selector(unbanPhrase(_:))), button("移除", #selector(removePhrase(_:)))])
        actions.spacing = 8; body.addArrangedSubview(actions)
        note("詞庫操作會立即儲存；系統詞移除學習紀錄後會恢復預設排序。")
        do { phrases = try model.manage(["op": "list_phrases"])["items"] as? [[String: Any]] ?? []; filterPhrases() }
        catch { showError(error) }
    }
    private func filterPhrases() {
        filtered = phrases.filter { phraseSearch.isEmpty || ($0["word"] as? String ?? "").contains(phraseSearch) || ($0["bopomofo"] as? String ?? "").contains(phraseSearch) }
        phraseTable?.reloadData()
    }
    func numberOfRows(in tableView: NSTableView) -> Int { filtered.count }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard filtered.indices.contains(row), let key = tableColumn?.identifier.rawValue else { return nil }
        let value: String
        if key == "state" {
            let state = filtered[row]["state"] as? Int ?? 0; value = state == 2 ? "封鎖" : (state == 1 ? "優先" : "一般")
        } else { value = filtered[row][key] as? String ?? "" }
        return NSTextField(labelWithString: value)
    }
    func tableViewSelectionDidChange(_ notification: Notification) {
        guard let row = phraseTable?.selectedRow, filtered.indices.contains(row) else { return }
        wordField?.stringValue = filtered[row]["word"] as? String ?? ""
        readingField?.stringValue = filtered[row]["bopomofo"] as? String ?? ""
    }
    @objc private func addPhrase(_ sender: Any?) {
        managePhrase(["op": "add_phrase", "word": wordField?.stringValue ?? "", "bopomofo": readingField?.stringValue ?? ""])
    }
    @objc private func editPhrase(_ sender: Any?) {
        guard let row = phraseTable?.selectedRow, filtered.indices.contains(row) else { return }
        let entry = filtered[row]
        let original: [String: Any] = ["word": entry["word"]!, "bopomofo": entry["bopomofo"]!, "boost": entry["boost"]!]
        managePhrase(["op": "edit_phrase", "original": original, "word": wordField?.stringValue ?? "", "bopomofo": readingField?.stringValue ?? ""])
    }
    @objc private func suggestReading(_ sender: Any?) {
        do { readingField?.stringValue = try model.manage(["op": "suggest_reading", "word": wordField?.stringValue ?? ""])["value"] as? String ?? "" }
        catch { showError(error) }
    }
    private func phraseAction(_ action: String) {
        guard let row = phraseTable?.selectedRow, filtered.indices.contains(row) else { return }
        managePhrase(["op": "phrase", "word": filtered[row]["word"]!, "bopomofo": filtered[row]["bopomofo"]!, "action": action])
    }
    @objc private func pinPhrase(_ sender: Any?) { phraseAction("Pin") }
    @objc private func banPhrase(_ sender: Any?) { phraseAction("Ban") }
    @objc private func unbanPhrase(_ sender: Any?) {
        guard let row = phraseTable?.selectedRow, filtered.indices.contains(row) else { return }
        phraseAction((filtered[row]["state"] as? Int ?? 0) == 2 ? "Unban" : "Unpin")
    }
    @objc private func removePhrase(_ sender: Any?) { phraseAction("Remove") }
    private func managePhrase(_ action: [String: Any]) {
        do { _ = try model.manage(action); showPage(); status.stringValue = "詞庫已更新" } catch { showError(error) }
    }
    private func learningView() {
        do {
            let stats = try model.manage(["op": "stats"])["items"] as? [[String: Any]] ?? []
            for day in stats.suffix(30).reversed() {
                let chars = (day["chars"] as? NSNumber)?.doubleValue ?? 0
                let corrections = (day["corrections"] as? NSNumber)?.doubleValue ?? 0
                let accuracy = ((day["accuracy"] as? NSNumber)?.doubleValue ?? 1) * 100
                body.addArrangedSubview(NSTextField(labelWithString: "\(day["day"] as? String ?? "")　\(Int(chars)) 字 · 修正 \(Int(corrections)) 次 · \(String(format: "%.1f", accuracy))%"))
            }
            if stats.isEmpty { note("尚無學習統計。") }
            let backups = try model.manage(["op": "backups"])["items"] as? [String] ?? []
            let picker = NSPopUpButton(); picker.addItems(withTitles: backups); backupPicker = picker
            let restore = button("從備份還原…", #selector(restoreBackup(_:))); restore.isEnabled = !backups.isEmpty
            let actions = NSStackView(views: [picker, restore, button("重新整理", #selector(refreshPage(_:)))]); actions.spacing = 8
            body.addArrangedSubview(actions)
            note("學習資料會定期儲存並按日備份，保留最近七份。還原後請切換到其他輸入法再切回。")
        } catch { showError(error) }
    }
    @objc private func restoreBackup(_ sender: Any?) {
        guard let day = backupPicker?.titleOfSelectedItem else { return }
        let alert = NSAlert(); alert.messageText = "還原到 \(day)？"; alert.informativeText = "目前的學習資料將被備份內容取代。"
        alert.addButton(withTitle: "還原"); alert.addButton(withTitle: "取消")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        do { _ = try model.manage(["op": "restore", "day": day]); status.stringValue = "已還原，請切換輸入法後繼續"; showPage() }
        catch { showError(error) }
    }
    private func refreshSyncStatus() {
        guard let label = syncStatusLabel else { return }
        do { label.stringValue = try model.manage(["op": "sync_status"])["message"] as? String ?? "尚未同步" }
        catch { label.stringValue = String(describing: error) }
    }
    private func locationView() {
        let syncLabel = NSTextField(wrappingLabelWithString: "尚未同步")
        syncLabel.preferredMaxLayoutWidth = 520
        body.addArrangedSubview(syncLabel); syncStatusLabel = syncLabel; refreshSyncStatus()
        note("使用系統 git 與既有 SSH／HTTPS 憑證。首次連線請先在終端機完成驗證；不會在輸入法內要求密碼。同步後按重新載入查看遠端設定，切換輸入法以套用。")
        note("資料位置：\(model.snapshot["data_root"] as? String ?? "")")
        let move = button("變更資料位置…", #selector(changeLocation(_:)))
        move.isEnabled = model.snapshot["location_locked"] as? Bool != true; body.addArrangedSubview(move)
        let sync = button("立即同步", #selector(syncNow(_:))); sync.isEnabled = model.supports("sync"); body.addArrangedSubview(sync)
    }
    @objc private func changeLocation(_ sender: Any?) {
        let panel = NSOpenPanel(); panel.canChooseDirectories = true; panel.canChooseFiles = false
        panel.canCreateDirectories = true; panel.allowsMultipleSelection = false
        panel.message = "會先儲存設定並保留原資料；若目的地已有輸入法資料，將改用該份資料。"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            collect(); try parseJSONDrafts(); try model.save()
            _ = try model.client.relocatePreferences(to: url.path)
            try model.reload(); jsonDrafts = [:]; showPage(); status.stringValue = "已變更資料位置，請切換輸入法"
        } catch { showError(error) }
    }
    @objc private func saveApiKey(_ sender: Any?) {
        let alert = NSAlert(); alert.messageText = "API Key"; alert.informativeText = "存入平台憑證庫；留空可清除。"
        let field = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 360, height: 24)); alert.accessoryView = field
        alert.addButton(withTitle: "儲存"); alert.addButton(withTitle: "取消")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        do { _ = try model.manage(["op": "write_api_key", "key": field.stringValue]); collect(); showPage(); status.stringValue = "憑證已更新" }
        catch { showError(error) }
    }
    private func refreshUpdateStatus() {
        let updates = UpdateManager.shared
        updateStatusLabel?.stringValue = updates.supported ? updates.message : "開發版或隔離測試不檢查更新"
        updateInstallButton?.isEnabled = updates.available != nil && !updates.busy
    }
    @objc private func checkUpdate(_ sender: Any?) { UpdateManager.shared.check(); refreshUpdateStatus() }
    @objc private func installUpdate(_ sender: Any?) { UpdateManager.shared.install(); refreshUpdateStatus() }
    @objc private func testAi(_ sender: Any?) {
        collect()
        guard let config = model.config["ai"] as? [String: Any] else { return }
        do {
            let key = try Credentials.read() ?? ""
            status.stringValue = "測試連線中…"
            (sender as? NSButton)?.isEnabled = false
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                let result = Result { try AIConnection.test(config: config, key: key) }
                DispatchQueue.main.async {
                    guard let self else { return }
                    (sender as? NSButton)?.isEnabled = true
                    switch result { case .success(let message): self.status.stringValue = "連線成功：" + message
                    case .failure(let error): self.showError(error) }
                }
            }
        } catch { showError(error) }
    }
    @objc private func syncNow(_ sender: Any?) {
        do { collect(); try parseJSONDrafts(); try model.save(); _ = try model.manage(["op": "sync_now"]); refreshSyncStatus(); status.stringValue = "已要求背景同步" }
        catch { showError(error) }
    }
    @objc private func refreshPage(_ sender: Any?) { collect(); showPage() }
    @objc private func openFolder(_ sender: Any?) { NSWorkspace.shared.open(model.client.configURL.deletingLastPathComponent()) }
    @objc private func editFile(_ sender: Any?) { NSWorkspace.shared.open(model.client.configURL) }
    @objc private func reloadSettings(_ sender: Any?) {
        do { try model.reload(); jsonDrafts = [:]; showPage(); status.stringValue = "已重新載入" } catch { showError(error) }
    }
    @objc private func saveSettings(_ sender: Any?) {
        window?.makeFirstResponder(nil); collect()
        do { try parseJSONDrafts(); try model.save(); status.stringValue = "已儲存" }
        catch { showError(error) }
    }
    private func showError(_ error: Error) {
        status.stringValue = "操作失敗"
        let alert = NSAlert(); alert.messageText = "無法完成設定操作"; alert.informativeText = String(describing: error)
        if let window, window.isVisible { alert.beginSheetModal(for: window) } else { NSLog("ZooType settings: %@", String(describing: error)) }
    }
}
