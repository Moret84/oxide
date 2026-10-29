import AppKit

/// A selection running across a feed's messages, as on Teams' page: the feed follows the drag itself and every message
/// text on screen shows its share. Within one text, the text view's own selection is used, so its menu and services still work.
@MainActor final class FeedSelection {
    /// One text of a message: its author line, then its body, split where images or cards sit between paragraphs.
    struct Unit: Hashable {
        static let authorLine = -1

        let message: String
        let segment: Int
    }

    struct Point {
        let unit: Unit
        let index: Int
    }

    /// Where each message stands in the feed, read when a drag starts.
    var rows: () -> [String: Int] = { [:] }
    /// The units of a feed row, in order; set by the feed, which owns the messages.
    var units: (Int) -> [(Unit, NSAttributedString)] = { _ in [] }

    private var anchor: Point?
    private var focus: Point?
    private var order: [String: Int] = [:]
    private let views = NSHashTable<MessageTextView>.weakObjects()

    /// The drag covers more than one text: Cmd+C copies through `text()` and highlights replace the views' selections.
    var spansUnits: Bool {
        guard let anchor, let focus else { return false }
        return anchor.unit != focus.unit
    }

    func register(_ view: MessageTextView) {
        views.add(view)
        show(in: view)
    }

    func begin(at point: Point) {
        clear()
        order = rows()
        anchor = point
        focus = point
    }

    func extend(to point: Point) {
        guard anchor != nil, focus?.unit != point.unit || focus?.index != point.index else { return }
        focus = point
        for view in views.allObjects { show(in: view) }
    }

    func clear() {
        anchor = nil
        focus = nil
        for view in views.allObjects {
            view.highlight(nil)
            view.setSelectedRange(NSRange(location: 0, length: 0))
        }
    }

    /// The selected part of every text between both ends, a blank line between messages.
    func text() -> String {
        guard let (start, end) = ends(), let first = order[start.unit.message], let last = order[end.unit.message] else {
            return ""
        }
        var parts: [String] = []
        var message: String?
        for row in first...last {
            for (unit, text) in units(row) {
                guard let range = range(for: unit, length: text.length), range.length > 0 else { continue }
                if message != nil && unit.message != message { parts.append("") }
                message = unit.message
                parts.append(MessageRichText.plain(text, range: range))
            }
        }
        return parts.joined(separator: "\n")
    }

    /// The part of `unit` selected, `length` being its text's length.
    func range(for unit: Unit, length: Int) -> NSRange? {
        guard let (start, end) = ends() else { return nil }
        guard let from = compare(unit, start.unit), let to = compare(unit, end.unit),
              from != .orderedAscending, to != .orderedDescending else { return nil }
        let lower = from == .orderedSame ? min(start.index, length) : 0
        let upper = to == .orderedSame ? min(end.index, length) : length
        return NSRange(location: lower, length: max(upper - lower, 0))
    }

    private func show(in view: MessageTextView) {
        guard let unit = view.unit, let anchor, spansUnits else {
            view.highlight(nil)
            if let anchor, let focus, view.unit == anchor.unit {
                let lower = min(anchor.index, focus.index)
                view.setSelectedRange(NSRange(location: lower, length: abs(focus.index - anchor.index)))
            }
            return
        }
        if unit == anchor.unit { view.setSelectedRange(NSRange(location: anchor.index, length: 0)) }
        view.highlight(range(for: unit, length: view.textStorage?.length ?? 0))
    }

    private func ends() -> (Point, Point)? {
        guard let anchor, let focus else { return nil }
        return compare(anchor.unit, focus.unit) == .orderedDescending
            || (anchor.unit == focus.unit && anchor.index > focus.index) ? (focus, anchor) : (anchor, focus)
    }

    private func compare(_ a: Unit, _ b: Unit) -> ComparisonResult? {
        if a.message == b.message { return a.segment == b.segment ? .orderedSame : a.segment < b.segment ? .orderedAscending : .orderedDescending }
        guard let rowA = order[a.message], let rowB = order[b.message] else { return nil }
        return rowA < rowB ? .orderedAscending : .orderedDescending
    }
}
