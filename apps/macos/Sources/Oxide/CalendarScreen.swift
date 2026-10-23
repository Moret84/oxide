import SwiftUI
import OxideCore

private enum CalendarMode: String, CaseIterable, Identifiable {
    case agenda = "Agenda", day = "Jour", week = "Semaine", month = "Mois"
    var id: Self { self }
    /// The raw value is stored in the user defaults: the label is separate.
    var label: String {
        switch self {
        case .agenda: L("ui.calendar_screen.agenda")
        case .day: L("ui.calendar_screen.day")
        case .week: L("ui.calendar_screen.week")
        case .month: L("ui.calendar_screen.month")
        }
    }
}

struct CalendarScreen: View {
    @Environment(Store.self) private var store
    @AppStorage("calendarMode") private var mode: CalendarMode = .week
    @State private var anchor = Date()
    @State private var destructive: String?
    /// For one date of a series: whether edits and answers go to the whole series.
    @State private var wholeSeries = false
    @State private var index = CalendarIndex()
    @FocusState private var focused: Bool
    private var model: CalendarView? { store.calendar }
    private var days: [Date] {
        let c = CalendarDates.local
        let start: Date
        let count: Int
        switch mode {
        case .agenda: start = c.startOfDay(for: anchor); count = 7
        case .day: start = c.startOfDay(for: anchor); count = 1
        case .week: start = CalendarDates.week(anchor); count = 7
        case .month: start = CalendarDates.week(c.dateInterval(of: .month, for: anchor)!.start); count = 42
        }
        return (0..<count).map { CalendarDates.adding(.day, $0, to: start) }
    }
    private var rangeKey: String { "\(mode.rawValue)-\(CalendarDates.iso(days[0]))" }
    private func send(_ action: CalendarAction) { store.send(.calendar(action: action)) }
    private func loadRange() {
        send(.range(start: CalendarDates.iso(days[0]), end: CalendarDates.iso(CalendarDates.adding(.day, 1, to: days.last!))))
    }
    private func newEvent(_ day: Date = Date()) {
        createEvent(at: CalendarDates.local.date(bySettingHour: 9, minute: 0, second: 0, of: day) ?? day)
    }
    private func createEvent(at start: Date) {
        guard model?.busy != true, model?.draft == nil else { return }
        send(.new(start: CalendarDates.iso(start), end: CalendarDates.iso(CalendarDates.adding(.minute, 30, to: start)), timeZone: TimeZone.current.identifier))
    }
    private func events(on day: Date) -> [CalendarIndex.Entry] { index.entries(on: day) }
    var body: some View {
        VStack(spacing: 0) {
            toolbar
            if let error = model?.error {
                ErrorBanner(text: error, retry: L("ui.calendar_screen.refresh"), disabled: model?.busy == true) { send(.refresh) }
            }
            Divider()
            HStack(spacing: 0) {
                if mode == .week || mode == .day {
                    rail
                    Divider()
                }
                mainArea
                sidePanel
            }
        }
        .background(Theme.surface)
        // ← and → change the period and T comes back to today, while no field has the focus.
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onKeyPress(.leftArrow) { shift(-1); return .handled }
        .onKeyPress(.rightArrow) { shift(1); return .handled }
        .onKeyPress(characters: CharacterSet(charactersIn: "tT")) { press in
            guard press.modifiers.isEmpty || press.modifiers == .shift else { return .ignored }
            anchor = Date()
            return .handled
        }
        .onChange(of: model?.events, initial: true) { _, events in index = CalendarIndex(events ?? []) }
        .onAppear {
            loadRange()
            focused = true
        }
        .onChange(of: rangeKey) { _, _ in loadRange() }
        .sheet(isPresented: Binding(get: { model?.draft != nil }, set: { if !$0 { send(.closeEditor) } })) {
            if let form = model?.draft { CalendarEditor(initial: form, editing: model?.editing == true) }
        }
        .confirmationDialog(destructive == "cancel" ? L("ui.calendar_screen.cancel_the_meeting_and_notify_attendees") : L("ui.calendar_screen.delete_this_event_from_your_calendar"), isPresented: Binding(get: { destructive != nil }, set: { if !$0 { destructive = nil } })) {
            Button(destructive == "cancel" ? L("ui.calendar_screen.cancel_meeting") : L("ui.calendar_screen.delete"), role: .destructive) {
                send(destructive == "cancel" ? .cancel(series: wholeSeries) : .delete(series: wholeSeries))
                destructive = nil
            }
        }
    }
    private var toolbar: some View {
        HStack(spacing: 12) {
            Text(L("ui.calendar_screen.calendar")).font(.figtree(22, .semibold))
            Button(L("ui.calendar_screen.today")) { anchor = Date() }.help(L("ui.calendar_screen.today_t"))
            Button { shift(-1) } label: { Image(systemName: "chevron.left") }.accessibilityLabel(L("ui.calendar_screen.previous_period")).help(L("ui.calendar_screen.previous_period_2"))
            Button { shift(1) } label: { Image(systemName: "chevron.right") }.accessibilityLabel(L("ui.calendar_screen.next_period")).help(L("ui.calendar_screen.next_period_2"))
            Text(period).font(.figtree(15, .semibold))
            Spacer(minLength: 4)
            SegmentedPicker(selection: $mode, options: CalendarMode.allCases, label: \.label)
                .accessibilityLabel(L("ui.calendar_screen.view"))
            Button { send(.refresh) } label: { Image(systemName: "arrow.clockwise") }.accessibilityLabel(L("ui.calendar_screen.refresh_calendar"))
            Button(L("ui.calendar_screen.new_event"), systemImage: "plus") { newEvent(anchor) }.buttonStyle(PrimaryButtonStyle()).disabled(model?.busy == true || model?.draft != nil)
        }.padding(16)
    }
    private var mainArea: some View {
        VStack(spacing: 0) {
            if model?.loading == true { ProgressView().controlSize(.small).padding(6) }
            switch mode {
            case .agenda:
                AgendaList(days: days, index: index, selected: model?.selected?.id, busy: model?.busy == true,
                           select: { send(.select(id: $0)) }, join: join, newEvent: newEvent)
            case .month: monthGrid
            case .day, .week: timeline
            }
            if mode != .agenda && model?.loading != true && model?.events.isEmpty == true {
                Text(L("ui.calendar_screen.no_events_in_this_period")).font(.figtree(12)).foregroundStyle(Theme.muted).padding(8)
            }
        }
    }
    /// The selected event; the agenda keeps its column even with nothing selected.
    @ViewBuilder private var sidePanel: some View {
        if model?.loadingDetail == true {
            Divider()
            ProgressView(L("ui.calendar_screen.loading_event")).frame(width: 330, height: 200)
        } else if let event = model?.selected {
            Divider()
            CalendarEventDetails(event: event, wholeSeries: $wholeSeries, destructive: $destructive, join: join).frame(width: 330)
        } else if mode == .agenda {
            Divider()
            VStack(spacing: 10) {
                Image(systemName: "calendar").font(.system(size: 28)).foregroundStyle(Theme.subtle)
                Text(L("ui.calendar_screen.select_an_event_to_see_its")).font(.figtree(12)).foregroundStyle(Theme.muted).multilineTextAlignment(.center)
            }.padding(24).frame(width: 330).frame(maxHeight: .infinity)
        }
    }
    private var period: String {
        guard mode == .agenda || mode == .week, let first = days.first, let last = days.last else {
            return mode == .day ? anchor.formatted(.dateTime.locale(L10n.locale).weekday(.wide).day().month(.wide).year()) : anchor.formatted(.dateTime.locale(L10n.locale).month(.wide).year())
        }
        let sameMonth = CalendarDates.local.isDate(first, equalTo: last, toGranularity: .month)
        return (sameMonth ? first.formatted(.dateTime.locale(L10n.locale).day()) : first.formatted(.dateTime.locale(L10n.locale).day().month(.abbreviated))) + " – " + last.formatted(.dateTime.locale(L10n.locale).day().month(.abbreviated).year())
    }
    /// Mini month, next meeting and legend beside the day and week timelines.
    private var rail: some View {
        let pending = index.all.filter { CalendarStatus($0.event) == .pending }.count
        return ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                MiniMonth(anchor: $anchor, shown: days) { !index.entries(on: $0).isEmpty }
                NextMeetingCard(entries: index.all, select: { send(.select(id: $0)) }, join: join)
                if pending > 0 {
                    Button { mode = .agenda } label: {
                        Label(Lp("ui.calendar_screen.to_respond_to", pending), systemImage: "envelope.badge")
                            .font(.figtree(12)).padding(.horizontal, 6).padding(.vertical, 4)
                    }.buttonStyle(GhostButtonStyle(accent: true))
                }
                CalendarLegend()
            }.padding(14)
        }
        .frame(width: 232)
        .background(Theme.background)
    }
    private func join(_ event: CalendarEventView) {
        store.send(.call(action: .prepareMeeting(url: event.joinUrl, title: event.title)))
    }
    private func shift(_ amount: Int) {
        anchor = CalendarDates.adding(mode == .month ? .month : .day, amount * (mode == .week || mode == .agenda ? 7 : 1), to: anchor)
    }
    private var monthGrid: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) { ForEach(Array(days.prefix(7)), id: \.self) { day in Text(day.formatted(.dateTime.locale(L10n.locale).weekday(.wide))).font(.figtree(12, .semibold)).frame(maxWidth: .infinity).padding(8) } }
            GeometryReader { size in
                ScrollView {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 0), count: 7), spacing: 0) {
                        ForEach(days, id: \.self) { day in
                            VStack(alignment: .leading, spacing: 4) {
                                HStack {
                                    Button { anchor = day; mode = .day } label: {
                                        Text(day.formatted(.dateTime.locale(L10n.locale).day())).font(.figtree(13, CalendarDates.local.isDateInToday(day) ? .bold : .regular))
                                            .foregroundStyle(CalendarDates.local.isDateInToday(day) ? Theme.accent : Theme.text)
                                    }.buttonStyle(.plain).pointerCursor()
                                    Spacer()
                                    Button { newEvent(day) } label: { Image(systemName: "plus").font(.system(size: 10)) }.buttonStyle(.plain).pointerCursor().accessibilityLabel(L("ui.calendar_screen.new_event_on_this_day"))
                                }
                                ForEach(events(on: day), id: \.event.id) { entry in eventButton(entry.event, start: entry.start) }
                                Spacer(minLength: 0)
                            }.padding(7).frame(maxWidth: .infinity, minHeight: max(95, size.size.height / 6), alignment: .topLeading)
                                .background(CalendarDates.local.component(.month, from: day) == CalendarDates.local.component(.month, from: anchor) ? Theme.surface : Theme.background)
                                .overlay(Rectangle().stroke(Theme.border, lineWidth: 0.5))
                        }
                    }
                }
            }
        }
    }
    private var timeline: some View {
        GeometryReader { geo in
            let columnWidth = max(mode == .day ? 400.0 : 130.0, (geo.size.width - 54) / CGFloat(days.count))
            ScrollView(.horizontal) {
                VStack(spacing: 0) {
                    HStack(alignment: .top, spacing: 0) {
                        Color.clear.frame(width: 54)
                        // Full height, so the day separators run from the hour grid up through the all-day band.
                        ForEach(days, id: \.self) { day in
                            dayHeader(day).padding(.bottom, 8)
                                .frame(width: columnWidth, alignment: .top).frame(maxHeight: .infinity, alignment: .top)
                                .background(columnTint(day))
                                .overlay(alignment: .leading) { Rectangle().fill(Theme.border).frame(width: 0.5) }
                        }
                    }.fixedSize(horizontal: false, vertical: true)
                    Divider()
                    ScrollViewReader { proxy in
                        ScrollView(.vertical) {
                            HStack(alignment: .top, spacing: 0) {
                                VStack(spacing: 0) {
                                    ForEach(0..<24) { hour in
                                        Text(Self.hourLabel(hour)).font(.figtree(10)).foregroundStyle(Theme.muted)
                                            .frame(width: 54, height: 56, alignment: .top).id(hour)
                                    }
                                }
                                ForEach(days, id: \.self) { day in dayTimeline(day).frame(width: columnWidth, height: 1344) }
                            }
                        }
                        // A legacy scroller would narrow the grid and centre it, out of line with the day headers.
                        .scrollIndicators(.never)
                        .onAppear { proxy.scrollTo(8, anchor: .top) }
                    }
                }.frame(width: 54 + columnWidth * CGFloat(days.count), height: geo.size.height)
            }.defaultScrollAnchor(.topLeading)
        }
    }
    private func dayTimeline(_ day: Date) -> some View {
        let layout = CalendarLayout.items(entries: events(on: day).filter { !$0.isBanner(on: day) }, day: day)
        return GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                DaySlots { slot in
                    let start = CalendarDates.local.date(bySettingHour: slot / 2, minute: slot % 2 * 30, second: 0, of: day)
                    createEvent(at: start ?? day)
                }
                ForEach(layout, id: \.event.id) { item in
                    eventButton(item.event, start: item.startDate, fill: true)
                        .frame(width: (geo.size.width - 4) / CGFloat(item.columns) - 3, height: max(24, item.end - item.start), alignment: .topLeading)
                        .offset(x: CGFloat(item.column) * (geo.size.width - 4) / CGFloat(item.columns) + 2, y: item.start)
                }
            }
            .background(columnTint(day))
            .overlay(alignment: .leading) { Rectangle().fill(Theme.border).frame(width: 0.5) }
            .overlay(alignment: .topLeading) {
                if CalendarDates.local.isDateInToday(day) { NowLine(day: day) }
            }
        }
    }
    private func dayHeader(_ day: Date) -> some View {
        let today = CalendarDates.local.isDateInToday(day)
        return VStack(spacing: 4) {
            Button { anchor = day; mode = .day } label: {
                VStack(spacing: 2) {
                    Text(day.formatted(.dateTime.locale(L10n.locale).weekday(.abbreviated))).font(.figtree(11, .semibold)).foregroundStyle(today ? Theme.accentText : Theme.muted)
                    Text(day.formatted(.dateTime.locale(L10n.locale).day())).font(.figtree(16, today ? .bold : .medium))
                        .foregroundStyle(today ? Color.white : Theme.text)
                        .frame(width: 30, height: 30)
                        .background(today ? Theme.accent : Color.clear, in: Circle())
                }.padding(.vertical, 6).contentShape(Rectangle())
            }.buttonStyle(.plain).pointerCursor().help(L("ui.calendar_screen.show_this_day"))
            ForEach(events(on: day).filter { $0.isBanner(on: day) }, id: \.event.id) { entry in
                bannerButton(entry.event).padding(.horizontal, 2)
            }
        }
    }
    private func columnTint(_ day: Date) -> Color {
        let c = CalendarDates.local
        if c.isDateInToday(day) { return Theme.accentSoft.opacity(0.35) }
        return c.isDateInWeekend(day) ? Theme.background : .clear
    }
    /// All-day and multi-day events: one line under the date, no time.
    private func bannerButton(_ e: CalendarEventView) -> some View {
        let status = CalendarStatus(e)
        return Button { send(.select(id: e.id)) } label: {
            Text((e.cancelled ? L("ui.calendar_screen.canceled") : "") + e.title).font(.figtree(11, .semibold)).strikethrough(e.cancelled).lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.leading, status == .accepted ? 7 : 5).padding(.trailing, 5).padding(.vertical, 2)
                .eventChip(status)
                .overlay { if model?.selected?.id == e.id { RoundedRectangle(cornerRadius: 5).strokeBorder(Theme.accent, lineWidth: 2) } }
                .contentShape(Rectangle())
        }.buttonStyle(.plain).pointerCursor().help(e.title + " · " + status.label).disabled(model?.busy == true)
    }
    /// `fill` stretches the block to the height the timeline gives it.
    private func eventButton(_ e: CalendarEventView, start: Date, fill: Bool = false) -> some View {
        let status = CalendarStatus(e)
        let selected = model?.selected?.id == e.id
        return Button { send(.select(id: e.id)) } label: {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 4) {
                    if e.form.online { Image(systemName: "video").font(.system(size: 9)) }
                    Text((e.cancelled ? L("ui.calendar_screen.canceled") : "") + e.title).font(.figtree(12, .semibold)).strikethrough(e.cancelled).lineLimit(2)
                }
                if !e.form.allDay { Text(CalendarText.time(start)).font(.figtree(10)) }
            }
            .frame(maxWidth: .infinity, maxHeight: fill ? .infinity : nil, alignment: .topLeading)
            .padding(.leading, status == .accepted ? 8 : 5).padding(.trailing, 5).padding(.vertical, 4)
            .eventChip(status)
            .overlay { if selected { RoundedRectangle(cornerRadius: 5).strokeBorder(Theme.accent, lineWidth: 2) } }
            .contentShape(Rectangle())
        }.buttonStyle(.plain).pointerCursor().help(e.title + " · " + status.label).disabled(model?.busy == true)
    }
    /// Labels already formatted, per language and hour.
    private static var hourLabels: [String: String] = [:]
    /// Hour of the day grid: "09:00" in French, "9 AM" in English.
    private static func hourLabel(_ hour: Int) -> String {
        let format = L("ui.calendar_screen.hour_format")
        let key = "\(L10n.locale.identifier)|\(format)|\(hour)"
        if let label = hourLabels[key] { return label }
        let formatter = DateFormatter()
        formatter.locale = L10n.locale
        formatter.dateFormat = format
        let date = Calendar.current.date(bySettingHour: hour, minute: 0, second: 0, of: Date()) ?? Date()
        let label = formatter.string(from: date)
        hourLabels[key] = label
        return label
    }
}

/// The 48 half hours of a day column as one view, which builds far faster than one view each:
/// clicking one opens a new event at that time.
private struct DaySlots: View {
    let create: (Int) -> Void
    @State private var hovered: Int?
    private static let height: CGFloat = 28

    var body: some View {
        // Read here rather than in the drawing, so a change of appearance redraws the lines.
        let border = Theme.border
        Canvas { context, size in
            for hour in 0..<24 {
                let line = CGRect(x: 0, y: CGFloat(hour) * Self.height * 2, width: size.width, height: 0.5)
                context.fill(Path(line), with: .color(border))
            }
        }
        .background(alignment: .top) {
            if let hovered {
                Rectangle().fill(Theme.accent.opacity(0.08)).frame(height: Self.height)
                    .offset(y: CGFloat(hovered) * Self.height)
            }
        }
        .contentShape(Rectangle())
        .onContinuousHover { phase in
            switch phase {
            case .active(let point): hovered = Self.slot(point.y)
            case .ended: hovered = nil
            }
        }
        .onTapGesture { point in create(Self.slot(point.y)) }
        .pointerCursor()
        .help(L("ui.calendar_screen.new_event"))
        .accessibilityChildren {
            VStack(spacing: 0) {
                ForEach(0..<48, id: \.self) { slot in
                    Rectangle().frame(height: Self.height)
                        .accessibilityAddTraits(.isButton)
                        .accessibilityLabel(L("ui.calendar_screen.new_event_at_this_time"))
                        .accessibilityAction { create(slot) }
                }
            }
        }
    }
    private static func slot(_ y: CGFloat) -> Int { min(max(Int(y / height), 0), 47) }
}

/// The current time across today's column (56 points per hour), moved every minute.
private struct NowLine: View {
    let day: Date
    var body: some View {
        TimelineView(.everyMinute) { context in
            let minutes = context.date.timeIntervalSince(day) / 60
            HStack(spacing: 0) {
                Circle().fill(Theme.danger).frame(width: 8, height: 8).offset(x: -4)
                Rectangle().fill(Theme.danger).frame(height: 1.5)
            }
            .offset(y: CGFloat(minutes) * 56 / 60 - 4)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}
