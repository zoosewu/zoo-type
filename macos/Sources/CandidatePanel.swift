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
    var compositionBar = false

    init(config: [String: Any] = [:]) {
        guard let theme = config["theme"] as? [String: Any] else { return }
        horizontal = theme["layout"] as? String == "horizontal"
        fontName = theme["font"] as? String ?? fontName
        fontSize = max(8, min(48, CGFloat((theme["font_size"] as? NSNumber)?.doubleValue ?? 16)))
        compositionBar = theme["composition_bar"] as? Bool ?? false
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
}

// 與 Windows 相同的字級比例、內距、顏色、數字欄與整列選取框。
final class CandidateView: NSView {
    struct Run { let text: String; let rect: NSRect; let font: NSFont; let color: NSColor }
    struct Hit { let rect: NSRect; let page: Int; let row: Int }
    private(set) var hits: [Hit] = []
    var onChoose: ((Int, Int) -> Void)?
    var onPage: ((Bool) -> Void)?
    private var runs: [Run] = []
    private(set) var selection: NSRect?
    private(set) var compositionCaret: NSRect?
    private(set) var compositionTarget: NSRect?
    private var theme = CandidateTheme()
    override var isFlipped: Bool { true }
    override var intrinsicContentSize: NSSize { frame.size }

    @discardableResult
    func update(_ candidates: CandidateDisplay, composition: CompositionDisplay? = nil,
                notice: String = "", theme: CandidateTheme) -> NSSize {
        let items = candidates.displayOnly ? [] : candidates.items
        let notice = candidates.displayOnly ? candidates.items.joined(separator: "  ") : notice
        let columns = candidates.columns, labels = candidates.labels
        let highlight = candidates.highlight, page = candidates.page
        let firstColumn = candidates.firstColumn, totalPages = candidates.totalPages
        self.theme = theme
        runs = []; hits = []; selection = nil
        compositionCaret = nil; compositionTarget = nil
        let font = theme.font
        let small = NSFont.systemFont(ofSize: theme.fontSize * 0.75)
        func width(_ text: String, _ font: NSFont) -> CGFloat {
            ceil((text as NSString).size(withAttributes: [.font: font]).width)
        }
        func add(_ text: String, x: CGFloat, y: CGFloat, font: NSFont, color: NSColor) {
            let height = ceil(font.ascender - font.descender + font.leading)
            runs.append(Run(text: text, rect: NSRect(x: x, y: y, width: width(text, font), height: height), font: font, color: color))
        }
        let textHeight = ceil(font.ascender - font.descender + font.leading)
        let numberHeight = ceil(small.ascender - small.descender + small.leading)
        let rowHeight = textHeight + 4
        let numberWidth = max(width("0", small), labels.map { width($0, small) }.max() ?? 0)
        var y: CGFloat = 4
        var contentWidth: CGFloat = 0
        if let composition, !composition.text.isEmpty {
            let preedit = composition.text, cursor = composition.cursor, target = composition.target
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: preedit, attributes: [.font: font]))
            let length = preedit.utf16.count
            func position(_ index: Int) -> CGFloat {
                12 + CTLineGetOffsetForStringIndex(line, index, nil)
            }
            compositionCaret = NSRect(x: position(max(0, min(cursor, length))), y: y + 2, width: 1.5, height: textHeight)
            if target.length > 0 && target.location <= length && target.length <= length - target.location {
                let left = position(target.location), right = position(NSMaxRange(target))
                compositionTarget = NSRect(x: min(left, right), y: y, width: abs(right - left), height: rowHeight)
            }
            add(preedit, x: 12, y: y + 2, font: font, color: theme.text)
            contentWidth = width(preedit, font) + 12
            y += rowHeight
        }
        if !columns.isEmpty {
            var x: CGFloat = 6
            var maxRows = 0
            for (columnIndex, column) in columns.enumerated() {
                let columnWidth = 12 + numberWidth + 8 + (column.map { width($0, font) }.max() ?? 0)
                for (i, item) in column.enumerated() {
                    let rowY = y + CGFloat(i) * rowHeight
                    hits.append(Hit(rect: NSRect(x: x, y: rowY, width: columnWidth, height: rowHeight), page: columnIndex + firstColumn, row: i))
                    if columnIndex + firstColumn == page && i == highlight {
                        selection = NSRect(x: x, y: rowY, width: columnWidth, height: rowHeight)
                    }
                    if columnIndex + firstColumn == page {
                        add(labels[i], x: x + 6, y: rowY + (rowHeight - numberHeight) / 2, font: small, color: theme.number)
                    }
                    add(item, x: x + 6 + numberWidth + 8, y: rowY + 2, font: font, color: theme.text)
                }
                maxRows = max(maxRows, column.count)
                x += columnWidth + 8
            }
            contentWidth = max(contentWidth, x - 8)
            y += CGFloat(maxRows) * rowHeight
        } else if theme.horizontal && !items.isEmpty {
            var x: CGFloat = 6
            for (i, item) in items.enumerated() {
                let num = labels[i]
                let nw = width(num, small)
                let cellWidth = 12 + nw + 4 + width(item, font)
                hits.append(Hit(rect: NSRect(x: x, y: y, width: cellWidth, height: rowHeight), page: page, row: i))
                if i == highlight { selection = NSRect(x: x, y: y, width: cellWidth, height: rowHeight) }
                add(num, x: x + 6, y: y + (rowHeight - numberHeight) / 2, font: small, color: theme.number)
                add(item, x: x + 6 + nw + 4, y: y + 2, font: font, color: theme.text)
                x += cellWidth
            }
            contentWidth = max(contentWidth, x)
            y += rowHeight
        } else if !items.isEmpty {
            let rowWidth = 12 + numberWidth + 8 + (items.map { width($0, font) }.max() ?? 0)
            contentWidth = max(contentWidth, rowWidth + 6)
            for (i, item) in items.enumerated() {
                let rowY = y + CGFloat(i) * rowHeight
                hits.append(Hit(rect: NSRect(x: 6, y: rowY, width: rowWidth, height: rowHeight), page: page, row: i))
                if i == highlight { selection = NSRect(x: 6, y: rowY, width: rowWidth, height: rowHeight) }
                let number = labels[i]
                add(number, x: 12 + numberWidth - width(number, small), y: rowY + (rowHeight - numberHeight) / 2, font: small, color: theme.number)
                add(item, x: 12 + numberWidth + 8, y: rowY + 2, font: font, color: theme.text)
            }
            y += CGFloat(items.count) * rowHeight
        }
        if !items.isEmpty && totalPages > 1 {
            let footer = "\(page + 1)/\(totalPages)"
            contentWidth = max(contentWidth, width(footer, small) + 18)
            add(footer, x: contentWidth - width(footer, small) - 6, y: y + 2, font: small, color: theme.number)
            y += numberHeight + 4
        }
        if !notice.isEmpty {
            add(notice, x: 12, y: y + 2, font: font, color: theme.text)
            contentWidth = max(contentWidth, width(notice, font) + 12)
            y += rowHeight
        }
        let size = NSSize(width: max(36, contentWidth + 6), height: y + 4)
        frame.size = size
        needsDisplay = true
        return size
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
        if let compositionTarget {
            theme.highlight.setFill()
            compositionTarget.fill()
        }
        if let selection {
            theme.highlight.setFill()
            NSBezierPath(roundedRect: selection, xRadius: 4, yRadius: 4).fill()
        }
        for run in runs {
            (run.text as NSString).draw(in: run.rect, withAttributes: [.font: run.font, .foregroundColor: run.color])
        }
        if let compositionCaret {
            theme.text.setFill()
            compositionCaret.fill()
        }
    }
}

final class CandidatePanel: NSPanel {
    private let candidateView = CandidateView()
    var onChoose: ((Int, Int) -> Void)? { didSet { candidateView.onChoose = onChoose } }
    var onPage: ((Bool) -> Void)? { didSet { candidateView.onPage = onPage } }
    private var noticeTimer: Timer?
    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        level = .popUpMenu; isFloatingPanel = true; hasShadow = true; hidesOnDeactivate = false
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        isOpaque = false; backgroundColor = .clear
        contentView = candidateView
    }
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    func show(result: EngineResult, caret: NSRect, theme: CandidateTheme, showComposition: Bool) {
        noticeTimer?.invalidate()
        let composition = showComposition && !result.composition.text.isEmpty ? result.composition : nil
        let candidates = result.candidatePage
        guard composition != nil || !candidates.items.isEmpty || !result.notice.isEmpty else { orderOut(nil); return }
        let size = candidateView.update(candidates, composition: composition, notice: result.notice, theme: theme)
        let screen = NSScreen.screens.first { $0.frame.intersects(caret) } ?? NSScreen.main
        let bounds = screen?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        var origin = NSPoint(x: caret.minX, y: caret.minY - size.height - 4)
        if origin.y < bounds.minY { origin.y = caret.maxY + 4 }
        origin.x = max(bounds.minX, min(origin.x, bounds.maxX - size.width))
        origin.y = max(bounds.minY, min(origin.y, bounds.maxY - size.height))
        setFrame(NSRect(origin: origin, size: size), display: true)
        orderFrontRegardless()
        if composition == nil && candidates.items.isEmpty && !candidates.displayOnly {
            noticeTimer = Timer.scheduledTimer(withTimeInterval: 0.8, repeats: false) { [weak self] _ in self?.orderOut(nil) }
        }
    }
}
