import AppKit
import Carbon
import CoreGraphics

struct InputSignals {
    var secure = false
    var hasTextContext = true
    var hasCaret = true
    var fullscreen = false
}

// 只收集訊號；判斷規則在共用層（EngineSession.notInput）
enum InputDetection {
    private static var observedAt = Date.distantPast
    private static var observedPID: pid_t = 0
    private static var coversScreen = false
    static func gather(hasTextContext: Bool, hasCaret: Bool) -> InputSignals {
        let app = NSWorkspace.shared.frontmostApplication
        let pid = app?.processIdentifier ?? 0
        if observedPID != pid || Date().timeIntervalSince(observedAt) > 0.4 {
            observedAt = Date(); observedPID = pid; coversScreen = false
            let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
            let screens = NSScreen.screens.compactMap { screen -> CGRect? in
                guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { return nil }
                return CGDisplayBounds(number.uint32Value)
            }
            for window in windows where (window[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid && (window[kCGWindowLayer as String] as? Int ?? 0) == 0 {
                guard let data = window[kCGWindowBounds as String] as? [String: Any],
                      let bounds = CGRect(dictionaryRepresentation: data as CFDictionary) else { continue }
                if screens.contains(where: { bounds.insetBy(dx: -2, dy: -2).contains($0) }) { coversScreen = true; break }
            }
        }
        return InputSignals(secure: IsSecureEventInputEnabled(), hasTextContext: hasTextContext, hasCaret: hasCaret, fullscreen: coversScreen)
    }
}
