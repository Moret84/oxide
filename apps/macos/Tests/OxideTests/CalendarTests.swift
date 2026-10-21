import XCTest
import OxideCore
@testable import Oxide

final class CalendarTests: XCTestCase {
    private var paris: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "Europe/Paris")!
        c.firstWeekday = 2
        c.minimumDaysInFirstWeek = 4
        return c
    }
    func testCalendarDaysUseLocalBoundariesAcrossBothClockChanges() {
        for (text, hours) in [("2026-03-28T23:00:00Z", 23), ("2026-10-24T22:00:00Z", 25)] {
            let start = CalendarDates.date(text)!
            let end = CalendarDates.adding(.day, 1, to: start, calendar: paris)
            XCTAssertEqual(end.timeIntervalSince(start), Double(hours * 3600))
            XCTAssertEqual(paris.component(.hour, from: end), 0)
            XCTAssertEqual(CalendarDates.date(CalendarDates.iso(end)), end)
        }
    }
    func testWeekStartsOnMondayAcrossMonthAndYearBoundaries() {
        let date = CalendarDates.date("2027-01-01T12:00:00Z")!
        let week = CalendarDates.week(date, calendar: paris)
        XCTAssertEqual(paris.component(.weekday, from: week), 2)
        XCTAssertEqual(paris.component(.year, from: week), 2026)
        XCTAssertEqual(paris.component(.day, from: week), 28)
    }
    private func event(_ id: String, day: Date, start: Int, end: Int) -> CalendarEventView {
        let form = CalendarDraft(subject: id, start: CalendarDates.iso(CalendarDates.adding(.minute, start, to: day)), end: CalendarDates.iso(CalendarDates.adding(.minute, end, to: day)), timeZone: TimeZone.current.identifier, allDay: false, location: "", description: "", online: false, attendees: [],
                                 recurrence: RecurrenceDraft(repeat: .never, unit: .day, interval: 1, weekdays: [], byWeekday: false, endDate: ""))
        return CalendarEventView(id: id, form: form, organizer: "", response: "", cancelled: false, recurring: false, occurrence: false, recurrenceText: "", canEdit: true, canCancel: false, canRespond: false, canDelete: true, joinUrl: "")
    }
    func testOverlappingEventsShareColumnsAndTouchingEventsReuseSpace() {
        let day = CalendarDates.local.startOfDay(for: CalendarDates.date("2026-10-15T12:00:00Z")!)
        let items = CalendarLayout.items([
            event("a", day: day, start: 540, end: 600),
            event("b", day: day, start: 570, end: 660),
            event("c", day: day, start: 600, end: 630),
            event("d", day: day, start: 660, end: 690),
        ], day: day)
        XCTAssertEqual(items.map(\.columns), [2, 2, 2, 1])
        XCTAssertEqual(items.map(\.column), [0, 1, 0, 0])
        XCTAssertEqual(items[0].end - items[0].start, 56, accuracy: 0.01)
    }
    func testThreeConcurrentEventsAndAnEmptyDay() {
        let day = CalendarDates.local.startOfDay(for: Date())
        XCTAssertTrue(CalendarLayout.items([], day: day).isEmpty)
        let items = CalendarLayout.items((0..<3).map { event("\($0)", day: day, start: 540 + $0, end: 660) }, day: day)
        XCTAssertEqual(Set(items.map(\.column)).count, 3)
        XCTAssertTrue(items.allSatisfy { $0.columns == 3 })
    }
    func testIndexMatchesThePerDayOverlapFilterSortedByStart() {
        let day = CalendarDates.local.startOfDay(for: CalendarDates.date("2026-10-15T12:00:00Z")!)
        let events = [
            event("late", day: day, start: 900, end: 960),
            event("early", day: day, start: 480, end: 540),
            event("overnight", day: day, start: 1380, end: 1500),
            event("ends-at-midnight", day: day, start: 1320, end: 1440),
            event("instant-at-midnight", day: day, start: 1440, end: 1440),
            event("three-days", day: day, start: -1440, end: 2880),
        ]
        let index = CalendarIndex(events)
        for offset in -2...3 {
            let current = CalendarDates.adding(.day, offset, to: day)
            let next = CalendarDates.adding(.day, 1, to: current)
            let expected = events.filter {
                let s = CalendarDates.date($0.form.start)!, e = CalendarDates.date($0.form.end)!
                return s < next && e > current
            }.sorted { $0.form.start < $1.form.start }.map(\.id)
            XCTAssertEqual(index.entries(on: current).map(\.event.id), expected, "day \(offset)")
        }
    }
}
