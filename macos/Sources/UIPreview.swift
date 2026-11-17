import AppKit

// Offscreen rendering uses the same native views as the installed input method.
func uiSelfTest(dictionary: String, output: URL) throws {
    let app = NSApplication.shared
    app.setActivationPolicy(.prohibited)
    let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: temporary) }
    setenv("ZCT_HOME", temporary.path, 1)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
    let runtime = try EngineRuntime(dictionary: dictionary)
    let settings = try SettingsWindowController(runtime: runtime)
    func snapshot(_ view: NSView, _ filename: String) throws {
        view.layoutSubtreeIfNeeded()
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else {
            throw EngineError(description: "無法繪製 UI 預覽")
        }
        view.cacheDisplay(in: view.bounds, to: rep)
        try rep.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(filename))
    }
    guard let window = settings.window, let content = window.contentView else { throw EngineError(description: "設定視窗不存在") }
    window.appearance = NSAppearance(named: .aqua)
    window.displayIfNeeded()
    try snapshot(content, "settings-general.png")
    settings.previewPage(2)
    window.displayIfNeeded()
    try snapshot(content, "settings-appearance.png")
    let panel = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 240, height: 200), styleMask: .borderless, backing: .buffered, defer: false)
    let candidate = CandidateView()
    panel.contentView = candidate
    var theme = CandidateTheme()
    let items = ["天氣", "天汽", "添氣", "填氣", "田契"]
    for (filename, horizontal, columns) in [("candidate-vertical.png", false, [] as [[String]]),
                                           ("candidate-horizontal.png", true, []),
                                           ("candidate-columns.png", false, [items, ["天器", "添器", "田氣"]])] {
        theme.horizontal = horizontal
        let size = candidate.update(items: items, pinned: [true, false, false, false, false], columns: columns,
                                    highlight: 0, page: 0, totalPages: 3, theme: theme)
        panel.setContentSize(size)
        try snapshot(candidate, filename)
    }
    guard let icon = NSImage(contentsOf: Bundle.main.resourceURL!.appendingPathComponent("menu.tiff")),
          icon.size == NSSize(width: 16, height: 16) else {
        throw EngineError(description: "選單圖示必須是 16 point，避免輸入法選單列被撐高")
    }
    print("macOS UI rendering / menu icon self-test passed: \(output.path)")
}
