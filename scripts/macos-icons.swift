import AppKit

// 用法：macos-icons.swift <Resources 資料夾> [標籤]
// 開發版傳 DEV：右下角加橘色標籤（與 Windows 的 make-icon.ps1 -Badge DEV 相同），選單列的小圖示只放第一個字母
let root = URL(fileURLWithPath: CommandLine.arguments[1])
let badge = CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : ""
try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
func representation(pixels: Int, points: CGFloat, color: NSColor, appIcon: Bool = false) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    // Match the Windows badge: an opaque, square white tile with black text.
    // Keep the tile in both menu-bar appearances rather than using a template glyph.
    let side = CGFloat(pixels)
    NSColor.white.setFill()
    NSRect(x: 0, y: 0, width: side, height: side).fill()
    let font = NSFont(name: "PingFangTC-Semibold", size: side * (appIcon ? 0.68 : 0.875)) ?? NSFont.boldSystemFont(ofSize: side * 0.875)
    let text = "動" as NSString
    let size = text.size(withAttributes: [.font: font])
    text.draw(at: NSPoint(x: (side - size.width) / 2, y: (side - size.height) / 2), withAttributes: [.font: font, .foregroundColor: color])
    if !badge.isEmpty {
        // 右下角的標籤（座標原點在左下）；字級依標籤寬高取小的，一行放得下
        let label = (appIcon ? badge : String(badge.prefix(1))) as NSString
        let box = NSRect(x: side * (appIcon ? 0.3 : 0.5), y: 0, width: side * (appIcon ? 0.7 : 0.5), height: side * (appIcon ? 0.34 : 0.45))
        NSColor(srgbRed: 232 / 255, green: 89 / 255, blue: 12 / 255, alpha: 1).setFill()
        box.fill()
        let em = min(box.height * 0.85, box.width / (CGFloat(label.length) * 0.68))
        let attributes: [NSAttributedString.Key: Any] = [.font: NSFont.boldSystemFont(ofSize: em), .foregroundColor: NSColor.white]
        let labelSize = label.size(withAttributes: attributes)
        label.draw(at: NSPoint(x: box.midX - labelSize.width / 2, y: box.midY - labelSize.height / 2), withAttributes: attributes)
    }
    NSGraphicsContext.restoreGraphicsState()
    rep.size = NSSize(width: points, height: points)
    return rep
}
for (name, color) in [("menu.tiff", NSColor.black), ("menu-dark.tiff", NSColor.black)] {
    let image = NSImage(size: NSSize(width: 16, height: 16))
    for pixels in [16, 32] { image.addRepresentation(representation(pixels: pixels, points: 16, color: color)) }
    try image.tiffRepresentation!.write(to: root.appendingPathComponent(name))
}
let app = representation(pixels: 1024, points: 1024, color: NSColor(calibratedWhite: 0.102, alpha: 1), appIcon: true)
try app.representation(using: .png, properties: [:])!.write(to: root.appendingPathComponent("app-icon.png"))
