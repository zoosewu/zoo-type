import AppKit

let root = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
func representation(pixels: Int, points: CGFloat, color: NSColor, appIcon: Bool = false) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    // Match the Windows badge: an opaque, square white tile with black text.
    // Keep the tile in both menu-bar appearances rather than using a template glyph.
    NSColor.white.setFill()
    NSRect(x: 0, y: 0, width: pixels, height: pixels).fill()
    let font = NSFont(name: "PingFangTC-Semibold", size: CGFloat(pixels) * (appIcon ? 0.68 : 0.875)) ?? NSFont.boldSystemFont(ofSize: CGFloat(pixels) * 0.875)
    let text = "動" as NSString
    let size = text.size(withAttributes: [.font: font])
    text.draw(at: NSPoint(x: (CGFloat(pixels) - size.width) / 2, y: (CGFloat(pixels) - size.height) / 2), withAttributes: [.font: font, .foregroundColor: color])
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
