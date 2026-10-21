import Foundation
import OxideCore

// Calendar arithmetic is local; only the transport uses UTC instants.
enum CalendarDates {
    // Built once: the month view parses every event's dates for each of its 42 days.
    private static let gregorian: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.firstWeekday = 2
        c.minimumDaysInFirstWeek = 4
        return c
    }()
    private static let formatter = ISO8601DateFormatter()
    private static let fractionalFormatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions.insert(.withFractionalSeconds)
        return f
    }()
    static var local: Calendar {
        var c = gregorian
        c.timeZone = .current
        return c
    }
    static func iso(_ d: Date) -> String { formatter.string(from: d) }
    static func date(_ s: String) -> Date? {
        formatter.date(from: s) ?? fractionalFormatter.date(from: s)
    }
    static func adding(_ component: Calendar.Component, _ amount: Int, to d: Date, calendar: Calendar = CalendarDates.local) -> Date {
        calendar.date(byAdding: component, value: amount, to: d) ?? d
    }
    static func week(_ d: Date, calendar: Calendar = CalendarDates.local) -> Date { calendar.dateInterval(of: .weekOfYear, for: d)!.start }
}

/// Events by local day, parsed and sorted once per change of the core's list instead of once per cell.
struct CalendarIndex {
    struct Entry {
        let event: CalendarEventView
        let start: Date
        let end: Date
        /// All-day, or running past `day` (a local start of day): drawn as a banner, without a time.
        func isBanner(on day: Date) -> Bool {
            event.form.allDay || start < day || end > CalendarDates.adding(.day, 1, to: day)
        }
    }
    private var byDay: [Date: [Entry]] = [:]
    /// Every event, by start.
    private(set) var all: [Entry] = []

    init(_ events: [CalendarEventView] = [], calendar: Calendar = CalendarDates.local) {
        let entries = events.compactMap { event -> Entry? in
            guard let s = CalendarDates.date(event.form.start), let e = CalendarDates.date(event.form.end) else { return nil }
            return Entry(event: event, start: s, end: e)
        }.sorted { $0.event.form.start < $1.event.form.start }
        all = entries
        // An event belongs to every day it overlaps: start < day + 1 and end > day.
        for entry in entries {
            var day = calendar.startOfDay(for: entry.start)
            while day < entry.end {
                byDay[day, default: []].append(entry)
                let next = CalendarDates.adding(.day, 1, to: day, calendar: calendar)
                guard next > day else { break }
                day = next
            }
        }
    }

    /// `day` is a local start of day.
    func entries(on day: Date) -> [Entry] { byDay[day] ?? [] }
}

// Greedy interval partitioning; each connected overlap group shares its column count.
enum CalendarLayout {
    struct Item { let event: CalendarEventView; let startDate: Date; var start: CGFloat; var end: CGFloat; var column: Int = 0; var columns: Int = 1 }
    static func items(_ events: [CalendarEventView], day: Date) -> [Item] {
        items(entries: events.compactMap { event in
            guard let start = CalendarDates.date(event.form.start), let end = CalendarDates.date(event.form.end) else { return nil }
            return CalendarIndex.Entry(event: event, start: start, end: end)
        }, day: day)
    }
    static func items(entries: [CalendarIndex.Entry], day: Date) -> [Item] {
        let c = CalendarDates.local
        let next = CalendarDates.adding(.day, 1, to: day)
        func position(_ d: Date) -> CGFloat {
            if d >= next { return 1344 }
            return CGFloat(c.component(.hour, from: d) * 60 + c.component(.minute, from: d)) * 56 / 60
        }
        var items = entries.map { entry -> Item in
            let y = position(entry.start)
            return Item(event: entry.event, startDate: entry.start, start: y, end: max(y + 24, position(entry.end)))
        }.sorted { $0.start < $1.start }
        var group = 0
        var ends: [CGFloat] = []
        var groupEnd: CGFloat = -1
        func finish(_ end: Int) { for i in group..<end { items[i].columns = max(1, ends.count) } }
        for i in items.indices {
            if items[i].start >= groupEnd { finish(i); group = i; ends = [] }
            let column = ends.firstIndex(where: { $0 <= items[i].start }) ?? ends.count
            if column == ends.count { ends.append(items[i].end) } else { ends[column] = items[i].end }
            items[i].column = column
            groupEnd = max(ends.max() ?? 0, items[i].end)
        }
        finish(items.count)
        return items
    }
}
