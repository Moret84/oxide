import SwiftUI
import OxideCore

/// How an event reads at a glance: filled once accepted, dashed while tentative, outlined until answered.
enum CalendarStatus: Equatable {
    case accepted, tentative, pending, declined, cancelled

    init(_ event: CalendarEventView) {
        if event.cancelled { self = .cancelled; return }
        switch event.response {
        case "Declined": self = .declined
        case "Tentative", "TentativelyAccepted": self = .tentative
        case "NotResponded", "None", "": self = event.canRespond ? .pending : .accepted
        default: self = .accepted
        }
    }
    var label: String {
        switch self {
        case .accepted: L("ui.calendar_parts.accepted")
        case .tentative: L("ui.calendar_parts.tentative")
        case .pending: L("ui.calendar_parts.not_responded")
        case .declined: L("ui.calendar_parts.declined")
        case .cancelled: L("ui.calendar_parts.canceled")
        }
    }
}

/// Colour bar shared by the timeline blocks, the agenda rows and the legend.
struct StatusMark: View {
    let status: CalendarStatus
    var width: CGFloat = 4
    var body: some View {
        let shape = RoundedRectangle(cornerRadius: width / 2)
        switch status {
        case .accepted: shape.fill(Theme.accent).frame(width: width)
        case .tentative: shape.strokeBorder(Theme.accent, style: StrokeStyle(lineWidth: 1.5, dash: [3, 2])).frame(width: width)
        case .pending: shape.strokeBorder(Theme.accent, lineWidth: 1.5).frame(width: width)
        case .declined, .cancelled: shape.fill(Theme.borderStrong).frame(width: width)
        }
    }
}

extension View {
    /// Background and outline of an event block for its answer.
    func eventChip(_ status: CalendarStatus) -> some View {
        let shape = RoundedRectangle(cornerRadius: 5)
        return self
            .foregroundStyle(status == .declined || status == .cancelled ? Theme.muted : Theme.accentText)
            .background {
                switch status {
                case .accepted: shape.fill(Theme.accentSoft)
                case .tentative: shape.fill(Theme.accentSoft.opacity(0.5))
                case .pending: shape.fill(Theme.surface)
                case .declined, .cancelled: shape.fill(Theme.input)
                }
            }
            .overlay {
                switch status {
                case .tentative: shape.strokeBorder(Theme.accent.opacity(0.7), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                case .pending: shape.strokeBorder(Theme.accent, lineWidth: 1)
                default: EmptyView()
                }
            }
            .overlay(alignment: .leading) {
                if status == .accepted { Rectangle().fill(Theme.accent).frame(width: 3) }
            }
            .clipShape(shape)
    }
}

enum CalendarText {
    static func time(_ date: Date) -> String { date.formatted(.dateTime.locale(L10n.locale).hour().minute()) }
    static func duration(_ seconds: TimeInterval) -> String {
        let minutes = max(0, Int((seconds / 60).rounded()))
        if minutes < 60 { return "\(minutes) min" }
        let rest = minutes % 60
        return rest == 0 ? L("ui.calendar_parts.duration_hours", ["h": minutes / 60])
            : L("ui.calendar_parts.duration_hours_minutes", ["h": minutes / 60, "m": String(format: "%02d", rest)])
    }
    /// "Dans 12 min", "En cours", "Demain à 10:30"…
    static func relative(start: Date, end: Date, now: Date) -> String {
        if start <= now && now < end { return L("ui.calendar_parts.now") }
        let minutes = Int(start.timeIntervalSince(now) / 60)
        if minutes < 1 { return L("ui.calendar_parts.starting_now") }
        if minutes < 60 { return L("ui.calendar_parts.in_min", ["minutes": minutes]) }
        let c = CalendarDates.local
        let time = time(start)
        if c.isDateInToday(start) { return L("ui.calendar_parts.today_at", ["time": time]) }
        if c.isDateInTomorrow(start) { return L("ui.calendar_parts.tomorrow_at", ["time": time]) }
        return start.formatted(.dateTime.locale(L10n.locale).weekday(.wide).day()) + L("ui.calendar_parts.at", ["time": time])
    }
    static func dayTitle(_ day: Date) -> String {
        let c = CalendarDates.local
        let date = day.formatted(.dateTime.locale(L10n.locale).weekday(.wide).day().month(.wide))
        if c.isDateInToday(day) { return L("ui.calendar_parts.today", ["date": date]) }
        if c.isDateInTomorrow(day) { return L("ui.calendar_parts.tomorrow", ["date": date]) }
        return date.prefix(1).uppercased() + date.dropFirst()
    }
}

/// Small month in the rail; its own arrows browse months without moving the main view.
struct MiniMonth: View {
    @Binding var anchor: Date
    let shown: [Date]
    let busy: (Date) -> Bool
    @State private var month = Date()

    private var cells: [Date] {
        let c = CalendarDates.local
        let start = CalendarDates.week(c.dateInterval(of: .month, for: month)!.start)
        return (0..<42).map { CalendarDates.adding(.day, $0, to: start) }
    }
    var body: some View {
        let c = CalendarDates.local
        let shownSet = Set(shown)
        VStack(spacing: 6) {
            HStack(spacing: 2) {
                Text(month.formatted(.dateTime.locale(L10n.locale).month(.wide).year())).font(.figtree(13, .semibold))
                Spacer()
                IconButton(symbol: "chevron.left", help: L("ui.calendar_parts.previous_month"), side: 22, glyph: 11) { month = CalendarDates.adding(.month, -1, to: month) }
                IconButton(symbol: "chevron.right", help: L("ui.calendar_parts.next_month"), side: 22, glyph: 11) { month = CalendarDates.adding(.month, 1, to: month) }
            }
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 0), count: 7), spacing: 2) {
                ForEach(Array(cells.prefix(7)), id: \.self) { day in
                    Text(day.formatted(.dateTime.locale(L10n.locale).weekday(.narrow))).font(.figtree(10, .semibold)).foregroundStyle(Theme.subtle).frame(height: 18)
                }
                ForEach(cells, id: \.self) { day in
                    let today = c.isDateInToday(day)
                    let inMonth = c.isDate(day, equalTo: month, toGranularity: .month)
                    Button { anchor = day } label: {
                        VStack(spacing: 1) {
                            Text(day.formatted(.dateTime.locale(L10n.locale).day()))
                                .font(.figtree(11, today ? .bold : .regular))
                                .foregroundStyle(today ? .white : inMonth ? Theme.text : Theme.subtle.opacity(0.6))
                                .frame(width: 20, height: 20)
                                .background(today ? Theme.accent : .clear, in: Circle())
                            Circle().fill(busy(day) && !today ? Theme.accent.opacity(0.6) : .clear).frame(width: 3, height: 3)
                        }
                        .frame(maxWidth: .infinity, minHeight: 26)
                        .background(shownSet.contains(day) ? Theme.accentSoft : .clear)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .pointerCursor()
                    .accessibilityLabel(day.formatted(Date.FormatStyle(date: .complete, time: .omitted, locale: L10n.locale)))
                }
            }
        }
        .onAppear { month = anchor }
        .onChange(of: anchor) { _, value in month = value }
    }
}

/// The next meeting still to come in the loaded period, with its join button.
struct NextMeetingCard: View {
    let entries: [CalendarIndex.Entry]
    let select: (String) -> Void
    let join: (CalendarEventView) -> Void

    var body: some View {
        TimelineView(.everyMinute) { context in
            let now = context.date
            let next = entries.first { entry in
                let status = CalendarStatus(entry.event)
                return !entry.event.form.allDay && entry.end > now && status != .declined && status != .cancelled
            }
            VStack(alignment: .leading, spacing: 6) {
                if let next {
                    Text(CalendarText.relative(start: next.start, end: next.end, now: now))
                        .font(.figtree(11, .semibold)).foregroundStyle(Theme.accentText)
                    Button { select(next.event.id) } label: {
                        Text(next.event.title).font(.figtree(13, .semibold)).foregroundStyle(Theme.text)
                            .lineLimit(2).multilineTextAlignment(.leading)
                    }.buttonStyle(.plain).pointerCursor()
                    Text(CalendarText.time(next.start) + " – " + CalendarText.time(next.end)
                         + (next.event.form.attendees.isEmpty ? "" : L("ui.calendar_parts.people", ["attendeesCount": next.event.form.attendees.count])))
                        .font(.figtree(11)).foregroundStyle(Theme.muted)
                    if !next.event.joinUrl.isEmpty {
                        Button { join(next.event) } label: {
                            Label(L("ui.calendar_parts.join"), systemImage: "video").frame(maxWidth: .infinity)
                        }
                        .buttonStyle(PrimaryButtonStyle())
                        .padding(.top, 2)
                    }
                } else {
                    Text(L("ui.calendar_parts.next_meeting")).font(.figtree(11, .semibold)).foregroundStyle(Theme.muted)
                    Text(L("ui.calendar_parts.nothing_else_in_this_period")).font(.figtree(12)).foregroundStyle(Theme.muted)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .card(radius: 10)
        }
    }
}

struct CalendarLegend: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach([CalendarStatus.accepted, .tentative, .pending], id: \.label) { status in
                HStack(spacing: 8) {
                    StatusMark(status: status, width: 10).frame(height: 10)
                    Text(status.label).font(.figtree(11)).foregroundStyle(Theme.muted)
                }
            }
        }
    }
}

/// Events listed day by day, sticky day headers, with the answer and join buttons on each row.
struct AgendaList: View {
    let days: [Date]
    let index: CalendarIndex
    let selected: String?
    let busy: Bool
    let select: (String) -> Void
    let join: (CalendarEventView) -> Void
    let newEvent: (Date) -> Void

    var body: some View {
        TimelineView(.everyMinute) { context in
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0, pinnedViews: [.sectionHeaders]) {
                        ForEach(days, id: \.self) { day in
                            Section {
                                let entries = index.entries(on: day)
                                if entries.isEmpty {
                                    HStack {
                                        Text(L("ui.calendar_parts.nothing_planned")).font(.figtree(12)).foregroundStyle(Theme.muted)
                                        Spacer()
                                        Button(L("ui.calendar_parts.add"), systemImage: "plus") { newEvent(day) }
                                            .buttonStyle(GhostButtonStyle(accent: true)).font(.figtree(12))
                                    }
                                    .padding(.horizontal, 16).padding(.vertical, 10)
                                } else {
                                    ForEach(entries, id: \.event.id) { entry in
                                        row(entry, day: day, now: context.date)
                                        Divider().padding(.leading, 86)
                                    }
                                }
                            } header: { header(day, entries: index.entries(on: day)) }
                            .id(day)
                        }
                    }
                }
                .onAppear {
                    if let today = days.first(where: { CalendarDates.local.isDateInToday($0) }) { proxy.scrollTo(today, anchor: .top) }
                }
            }
        }
    }

    private func header(_ day: Date, entries: [CalendarIndex.Entry]) -> some View {
        let next = CalendarDates.adding(.day, 1, to: day)
        let meetings = entries.filter { !$0.event.form.allDay && !$0.event.cancelled && $0.start >= day && $0.end <= next }
        let total = meetings.reduce(0) { $0 + $1.end.timeIntervalSince($1.start) }
        let today = CalendarDates.local.isDateInToday(day)
        return HStack {
            Text(CalendarText.dayTitle(day)).font(.figtree(13, .semibold)).foregroundStyle(today ? Theme.accentText : Theme.textSoft)
            Spacer()
            if !meetings.isEmpty {
                Text(Lp("ui.calendar_parts.meetings_total", meetings.count, ["total": CalendarText.duration(total)]))
                    .font(.figtree(11)).foregroundStyle(Theme.muted)
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 8)
        .background(Theme.background)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.border).frame(height: 0.5) }
    }

    private func row(_ entry: CalendarIndex.Entry, day: Date, now: Date) -> some View {
        let e = entry.event
        let status = CalendarStatus(e)
        let fullDay = entry.isBanner(on: day)
        let past = entry.end <= now
        let soon = !fullDay && !past && entry.start.timeIntervalSince(now) < 15 * 60
        let isSelected = selected == e.id
        let details = [e.form.location, e.form.online ? L("ui.calendar_parts.teams_meeting") : "",
                       e.form.attendees.isEmpty ? "" : L("ui.calendar_parts.attendees", ["attendeesCount": e.form.attendees.count])]
            .filter { !$0.isEmpty }.joined(separator: " · ")
        return Button { select(e.id) } label: {
            HStack(alignment: .center, spacing: 12) {
                VStack(alignment: .trailing, spacing: 2) {
                    Text(fullDay ? L("ui.calendar_parts.all_day") : CalendarText.time(entry.start)).font(.figtree(12, .semibold)).foregroundStyle(Theme.textSoft)
                    if !fullDay { Text(CalendarText.duration(entry.end.timeIntervalSince(entry.start))).font(.figtree(11)).foregroundStyle(Theme.muted) }
                }
                .frame(width: 58, alignment: .trailing)
                StatusMark(status: status).frame(minHeight: 30)
                VStack(alignment: .leading, spacing: 3) {
                    Text(e.title).font(.figtree(13, .semibold)).foregroundStyle(status == .cancelled || status == .declined ? Theme.muted : Theme.text)
                        .strikethrough(status == .cancelled).lineLimit(2).multilineTextAlignment(.leading)
                    if soon {
                        Text(CalendarText.relative(start: entry.start, end: entry.end, now: now)).font(.figtree(11, .semibold)).foregroundStyle(Theme.accentText)
                    } else if !details.isEmpty {
                        Text(details).font(.figtree(11)).foregroundStyle(Theme.muted).lineLimit(1)
                    }
                }
                Spacer(minLength: 8)
                if soon && !e.joinUrl.isEmpty && status != .cancelled {
                    Button { join(e) } label: { Label(L("ui.calendar_parts.join"), systemImage: "video") }.buttonStyle(PrimaryButtonStyle())
                } else if status == .pending || status == .cancelled {
                    Text(status.label).font(.figtree(11, .semibold))
                        .foregroundStyle(status == .cancelled ? Theme.danger : Theme.accentText)
                        .padding(.horizontal, 8).padding(.vertical, 3)
                        .background(status == .cancelled ? Theme.errorBackground : Theme.accentSoft, in: Capsule())
                }
            }
            .padding(.horizontal, 16).padding(.vertical, 9)
            .opacity(past && !isSelected ? 0.55 : 1)
            .background(isSelected ? Theme.accentSoft : soon ? Theme.accentSoft.opacity(0.4) : .clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .pointerCursor()
        .disabled(busy)
        .help(e.title)
    }
}

/// Initials in a coloured circle; attendees only come with an address, not an id to fetch a photo.
struct AttendeeBadge: View {
    let person: CalendarPerson
    var body: some View {
        let name = person.name.isEmpty ? person.address : person.name
        let words = name.split(whereSeparator: { $0.isWhitespace || $0 == "." || $0 == "@" })
        let initials = words.count > 1 ? "\(words[0].prefix(1))\(words[1].prefix(1))" : String(name.prefix(2))
        Text(initials.uppercased())
            .font(.figtree(10, .semibold)).foregroundStyle(.white)
            .frame(width: 26, height: 26)
            .background(Theme.avatar(person.address.lowercased()), in: Circle())
            .accessibilityHidden(true)
    }
}

extension CalendarEventView {
    /// The subject, or a placeholder for an event without one.
    var title: String {
        form.subject.trimmingCharacters(in: .whitespaces).isEmpty ? L("ui.calendar_parts.no_title") : form.subject
    }
}
