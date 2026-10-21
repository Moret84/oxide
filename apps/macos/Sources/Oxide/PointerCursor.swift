import AppKit
import SwiftUI

/// Cursor rects follow the control and vanish with it, without touching the global cursor stack when a menu closes.
extension View {
    func pointerCursor() -> some View { background(PointerCursorArea()) }
}
private struct PointerCursorArea: NSViewRepresentable {
    @Environment(\.isEnabled) private var enabled
    func makeNSView(context: Context) -> CursorView { CursorView() }
    func updateNSView(_ view: CursorView, context: Context) {
        if view.enabled != enabled {
            view.enabled = enabled
            view.window?.invalidateCursorRects(for: view)
        }
    }
    final class CursorView: NSView {
        var enabled = true
        private var tracking: NSTrackingArea?
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func resetCursorRects() { if enabled { addCursorRect(bounds, cursor: .pointingHand) } }
        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            guard tracking == nil else { return }
            let area = NSTrackingArea(rect: .zero, options: [.activeInKeyWindow, .inVisibleRect, .mouseEnteredAndExited, .cursorUpdate], owner: self)
            addTrackingArea(area)
            tracking = area
        }
        override func mouseEntered(with event: NSEvent) { if enabled { NSCursor.pointingHand.set() } }
        override func cursorUpdate(with event: NSEvent) { if enabled { NSCursor.pointingHand.set() } }
        override func mouseExited(with event: NSEvent) { NSCursor.arrow.set() }
    }
}
