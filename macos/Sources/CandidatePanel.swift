import AppKit
import CoreText

extension Notification.Name {
    static let zctSettingsChanged = Notification.Name("ZooTypeSettingsChanged")
    static let zctOpenSettings = Notification.Name("ZooTypeOpenSettings")
}

struct CandidateTheme {
    var horizontal = false
    var fontName = "PingFang TC"
    var fontSize: CGFloat = 16
    var background = NSColor.white
    var text = NSColor(calibratedWhite: 0.102, alpha: 1)
    var number = NSColor(calibratedWhite: 0.478, alpha: 1)
    var highlight = NSColor(calibratedRed: 0.8, green: 0.894, blue: 0.969, alpha: 1)

    init(config: [String: Any] = [:]) {
        guard let theme = config["theme"] as? [String: Any] else { return }
        horizontal = theme["layout"] as? String == "horizontal"
        fontName = theme["font"] as? String ?? fontName
        fontSize = max(8, min(48, CGFloat((theme["font_size"] as? NSNumber)?.doubleValue ?? 16)))
        background = Self.color(theme["background"] as? String) ?? background
        text = Self.color(theme["text"] as? String) ?? text
        number = Self.color(theme["number"] as? String) ?? number
        highlight = Self.color(theme["highlight"] as? String) ?? highlight
    }
    static func color(_ hex: String?) -> NSColor? {
        guard let hex, let value = UInt32(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) else { return nil }
        return NSColor(calibratedRed: CGFloat((value >> 16) & 255) / 255,
                       green: CGFloat((value >> 8) & 255) / 255, blue: CGFloat(value & 255) / 255, alpha: 1)
    }
    var font: NSFont { NSFont(name: fontName, size: fontSize) ?? NSFont.systemFont(ofSize: fontSize) }
    /// 數字：同一個字型，字級比例與 Windows 共用
    var numberFont: NSFont {
        let size = fontSize * CGFloat(zct_number_ratio())
        return NSFont(name: fontName, size: size) ?? NSFont.systemFont(ofSize: size)
    }
}

// 排版在共用層（zct_core::popup，與 Windows 相同）：這裡只量文字、照結果畫。
private struct PopupRect: Decodable {
    let x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat
    var rect: NSRect { NSRect(x: x, y: y, width: w, height: h) }
}
private struct PopupRun: Decodable { let text: String; let x: CGFloat; let y: CGFloat; let font: String; let ink: String }
private struct PopupHit: Decodable { let rect: PopupRect; let page: Int; let row: Int }
private struct PopupPart: Decodable {
    let runs: [PopupRun]; let highlights: [PopupRect]; let caret: PopupRect?; let hits: [PopupHit]
}
private struct PopupLayout: Decodable {
    let width: CGFloat; let height: CGFloat
    let composition: PopupPart; let candidates: PopupPart; let notice: PopupPart
}

/// 共用排版量文字用：候選字與數字的字型
private final class PopupFonts {
    let text: NSFont, number: NSFont
    init(text: NSFont, number: NSFont) { self.text = text; self.number = number }
    func font(_ kind: UInt32) -> NSFont { kind == 0 ? text : number }
    static func height(_ font: NSFont) -> CGFloat { ceil(font.ascender - font.descender + font.leading) }
}

final class CandidateView: NSView {
    struct Run { let text: String; let point: NSPoint; let font: NSFont; let color: NSColor }
    struct Hit { let rect: NSRect; let page: Int; let row: Int }
    private(set) var hits: [Hit] = []
    var onChoose: ((Int, Int) -> Void)?
    var onPage: ((Bool) -> Void)?
    private(set) var runs: [Run] = []
    private var highlights: [NSRect] = []
    private(set) var selection: NSRect?
    private(set) var compositionCaret: NSRect?
    private(set) var compositionTarget: NSRect?
    private var theme = CandidateTheme()
    override var isFlipped: Bool { true }
    override var intrinsicContentSize: NSSize { frame.size }

    @discardableResult
    func update(_ candidates: CandidateDisplay, composition: CompositionDisplay? = nil,
                notice: String = "", theme: CandidateTheme) -> NSSize {
        self.theme = theme
        runs = []; hits = []; highlights = []; selection = nil
        compositionCaret = nil; compositionTarget = nil
        var request: [String: Any] = ["horizontal": theme.horizontal, "notice": notice]
        if !candidates.items.isEmpty || !candidates.columns.isEmpty {
            request["page"] = [
                "items": candidates.items, "page": candidates.page, "total_pages": candidates.totalPages,
                "highlight": candidates.highlight ?? candidates.items.count,
                "pinned": candidates.labels.map { $0.hasSuffix("★") },
                "columns": candidates.columns, "first_column": candidates.firstColumn,
                "display_only": candidates.displayOnly
            ] as [String: Any]
        }
        if let composition, !composition.text.isEmpty {
            var value: [String: Any] = ["text": composition.text, "cursor": composition.cursor]
            if composition.target.length > 0 { value["target"] = [composition.target.location, composition.target.length] }
            request["composition"] = value
        }
        let fonts = PopupFonts(text: theme.font, number: theme.numberFont)
        guard let data = try? JSONSerialization.data(withJSONObject: request),
              let layout = Self.layout(String(decoding: data, as: UTF8.self), fonts: fonts) else {
            frame.size = .zero; needsDisplay = true; return .zero
        }
        for part in [layout.composition, layout.candidates, layout.notice] {
            for run in part.runs {
                runs.append(Run(text: run.text, point: NSPoint(x: run.x, y: run.y),
                                font: run.font == "number" ? fonts.number : fonts.text,
                                color: run.ink == "dim" ? theme.number : theme.text))
            }
            highlights += part.highlights.map(\.rect)
        }
        compositionCaret = layout.composition.caret?.rect
        compositionTarget = layout.composition.highlights.first?.rect
        selection = layout.candidates.highlights.first?.rect
        hits = layout.candidates.hits.map { Hit(rect: $0.rect.rect, page: $0.page, row: $0.row) }
        let size = NSSize(width: ceil(layout.width), height: ceil(layout.height))
        frame.size = size
        needsDisplay = true
        return size
    }

    private static func layout(_ request: String, fonts: PopupFonts) -> PopupLayout? {
        let measure: ZctMeasure = { context, text, font, width, height in
            guard let context, let text, let width, let height else { return }
            let fonts = Unmanaged<PopupFonts>.fromOpaque(context).takeUnretainedValue()
            let f = fonts.font(font)
            width.pointee = Float(ceil((String(cString: text) as NSString).size(withAttributes: [.font: f]).width))
            height.pointee = Float(PopupFonts.height(f))
        }
        let context = Unmanaged.passUnretained(fonts).toOpaque()
        guard let raw = request.withCString({ zct_popup_layout($0, measure, context) }) else {
            NSLog("ZooType popup layout: %@", EngineError.current.description); return nil
        }
        defer { zct_string_free(raw) }
        return try? JSONDecoder().decode(PopupLayout.self, from: Data(String(cString: raw).utf8))
    }

    override var acceptsFirstResponder: Bool { false }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if let hit = hits.first(where: { $0.rect.contains(point) }) { onChoose?(hit.page, hit.row) }
    }
    override func scrollWheel(with event: NSEvent) {
        if !hits.isEmpty && abs(event.scrollingDeltaY) >= 1 { onPage?(event.scrollingDeltaY > 0) }
    }
    override func draw(_ dirtyRect: NSRect) {
        theme.background.setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5), xRadius: 6, yRadius: 6).fill()
        theme.number.withAlphaComponent(0.2).setStroke()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5), xRadius: 6, yRadius: 6).stroke()
        theme.highlight.setFill()
        let radius = CGFloat(zct_highlight_radius())
        for rect in highlights { NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius).fill() }
        for run in runs {
            (run.text as NSString).draw(at: run.point, withAttributes: [.font: run.font, .foregroundColor: run.color])
        }
        if let compositionCaret {
            theme.text.setFill()
            compositionCaret.fill()
        }
    }
}

/// 一個彈出視窗，只放一種內容
final class PopupPanel: NSPanel {
    let view = CandidateView()
    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        level = .popUpMenu; isFloatingPanel = true; hasShadow = true; hidesOnDeactivate = false
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        isOpaque = false; backgroundColor = .clear
        contentView = view
    }
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    /// 位置規則與 Windows 共用：預設在 `anchor` 下方，放不下改上方，不超出螢幕；`above` 優先放上方
    func show(size: NSSize, at window: ZctPopupWindow) {
        guard size.width > 0 else { orderOut(nil); return }
        let anchor = NSRect(x: window.anchor.x, y: window.anchor.y, width: window.anchor.width, height: window.anchor.height), above = window.above
        let screen = NSScreen.screens.first { $0.frame.intersects(anchor) } ?? NSScreen.main
        let bounds = screen?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        let origin = zct_popup_place(anchor.minX, anchor.minY, anchor.width, anchor.height, size.width, size.height,
                                     bounds.minX, bounds.minY, bounds.width, bounds.height, above)
        setFrame(NSRect(origin: NSPoint(x: origin.x, y: origin.y), size: size), display: true)
        orderFrontRegardless()
    }
}

/// 和 Windows 一樣分成三個視窗：組字提示列、候選窗（含打字時的注音小視窗）、提示。
/// 各自出現、消失，不會互相推擠；顯示哪幾個、放在哪裡由共用層決定（zct_core::popup::Popups）
final class CandidatePanel {
    let bar = PopupPanel(), candidates = PopupPanel(), hint = PopupPanel()
    var onChoose: ((Int, Int) -> Void)? { didSet { candidates.view.onChoose = onChoose } }
    var onPage: ((Bool) -> Void)? { didSet { candidates.view.onPage = onPage } }
    private var hintTimer: Timer?
    private static let empty = CandidateDisplay(items: [], labels: [], highlight: 0, page: 0, totalPages: 0)
    var isVisible: Bool { bar.isVisible || candidates.isVisible || hint.isVisible }
    func orderOut(_ sender: Any?) {
        hintTimer?.invalidate()
        for panel in [bar, candidates, hint] { panel.orderOut(sender) }
    }
    /// `plan` 的 action：0 收起、1 顯示、2 不動（提示自己計時消失，之後的按鍵不會提早收掉）
    func show(result: EngineResult, plan: ZctPopupPlan, theme: CandidateTheme) {
        switch plan.bar.action {
        case 1: bar.show(size: bar.view.update(Self.empty, composition: result.composition, theme: theme), at: plan.bar)
        case 0: bar.orderOut(nil)
        default: break
        }
        switch plan.candidates.action {
        case 1: candidates.show(size: candidates.view.update(result.candidatePage, theme: theme), at: plan.candidates)
        case 0: candidates.orderOut(nil)
        default: break
        }
        switch plan.hint.action {
        case 1:
            hintTimer?.invalidate()
            hint.show(size: hint.view.update(Self.empty, notice: result.notice, theme: theme), at: plan.hint)
            hintTimer = Timer.scheduledTimer(withTimeInterval: zct_hint_seconds(), repeats: false) { [weak self] _ in self?.hint.orderOut(nil) }
        case 0: hintTimer?.invalidate(); hint.orderOut(nil)
        default: break
        }
    }
}
