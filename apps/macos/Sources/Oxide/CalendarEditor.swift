import SwiftUI
import OxideCore

struct CalendarEditor: View {
    @Environment(Store.self) private var store
    @State private var form: CalendarDraft
    @State private var query = ""
    @State private var optional = false
    @State private var pendingDraft: Task<Void, Never>?
    let editing: Bool
    init(initial: CalendarDraft, editing: Bool) { _form = State(initialValue: initial); self.editing = editing }
    private var model: CalendarView? { store.calendar }
    private func send(_ action: CalendarAction) { store.send(.calendar(action: action)) }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(editing ? L("ui.calendar_screen.edit_event") : L("ui.calendar_screen.new_event")).font(.figtree(22, .semibold))
            if let error = model?.error { Text(error).font(.figtree(12)).foregroundStyle(Theme.errorText).textSelection(.enabled) }
            ScrollView {
                Form {
                    TextField(L("ui.calendar_screen.title"), text: $form.subject)
                    Toggle(L("ui.calendar_screen.all_day_2"), isOn: $form.allDay).onChange(of: form.allDay) { _, allDay in
                        if allDay { let start = CalendarDates.local.startOfDay(for: CalendarDates.date(form.start) ?? Date()); form.start = CalendarDates.iso(start); form.end = CalendarDates.iso(CalendarDates.adding(.day, 1, to: start)) }
                    }
                    DatePicker(L("ui.calendar_screen.start"), selection: dateBinding(start: true), displayedComponents: form.allDay ? [.date] : [.date, .hourAndMinute])
                    DatePicker(L("ui.calendar_screen.end"), selection: dateBinding(start: false), displayedComponents: form.allDay ? [.date] : [.date, .hourAndMinute])
                    Text(L("ui.calendar_screen.time_zone", ["identifier": TimeZone.current.identifier])).font(.figtree(11)).foregroundStyle(Theme.muted)
                    TextField(L("ui.calendar_screen.location"), text: $form.location)
                    Toggle(L("ui.calendar_screen.teams_meeting"), isOn: $form.online).disabled(editing)
                    if model?.editingOccurrence != true { RepeatField(recurrence: $form.recurrence, start: CalendarDates.date(form.start) ?? Date()) }
                    Text(L("ui.calendar_screen.description")).font(.figtree(12))
                    TextEditor(text: $form.description).frame(minHeight: 90).overlay(RoundedRectangle(cornerRadius: 4).stroke(Theme.border))
                    Section(L("ui.calendar_screen.attendees")) {
                        HStack {
                            // Searches as the name is typed, once typing pauses.
                            TextField(L("ui.calendar_screen.name_or_email_address"), text: $query)
                                .onSubmit { searchOrAdd() }
                                .task(id: query) {
                                    let typed = query.trimmingCharacters(in: .whitespaces)
                                    guard typed.count >= 2, !typed.contains("@") else {
                                        send(.searchPeople(query: ""))
                                        return
                                    }
                                    try? await Task.sleep(for: .milliseconds(250))
                                    guard !Task.isCancelled else { return }
                                    send(.searchPeople(query: typed))
                                }
                            Button(L("ui.calendar_screen.add")) { add(CalendarPerson(address: query.trimmingCharacters(in: .whitespaces), name: "", optional: optional)) }.disabled(!query.contains("@"))
                        }
                        Toggle(L("ui.calendar_screen.optional_attendee"), isOn: $optional)
                        if model?.searching == true { ProgressView().controlSize(.small) }
                        ForEach(Array((model?.people ?? []).enumerated()), id: \.offset) { _, person in
                            Button("\(person.name) · \(person.address)") { add(person) }.buttonStyle(.link)
                        }
                        ForEach(Array(form.attendees.enumerated()), id: \.offset) { index, person in
                            HStack {
                                Text(person.name.isEmpty ? person.address : "\(person.name) · \(person.address)").lineLimit(1)
                                Text(person.optional ? L("ui.calendar_screen.optional") : L("ui.calendar_screen.required")).foregroundStyle(Theme.muted)
                                Spacer()
                                Button { form.attendees.remove(at: index) } label: { Image(systemName: "minus.circle") }.accessibilityLabel(L("ui.calendar_screen.remove", ["address": person.address]))
                            }
                        }
                    }
                }.formStyle(.grouped)
            }.disabled(model?.busy == true)
            HStack {
                Button(L("ui.calendar_screen.close")) { pendingDraft?.cancel(); send(.closeEditor) }.keyboardShortcut(.cancelAction).disabled(model?.busy == true)
                Spacer()
                if model?.busy == true { ProgressView().controlSize(.small) }
                Button(editing ? L("ui.calendar_screen.save") : L("ui.calendar_screen.create")) { pendingDraft?.cancel(); send(.draft(value: form)); send(.save) }.buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction).disabled(model?.busy == true || model?.uncertain == true || form.subject.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }.padding(24).frame(width: 660, height: 720)
            .onChange(of: form) { _, value in scheduleDraft(value) }
            .onDisappear { pendingDraft?.cancel() }
            .interactiveDismissDisabled(model?.busy == true)
    }
    /// Typing sends the draft to the core once it pauses; saving sends it straight away.
    private func scheduleDraft(_ value: CalendarDraft) {
        pendingDraft?.cancel()
        pendingDraft = Task {
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            send(.draft(value: value))
        }
    }
    private func dateBinding(start: Bool) -> Binding<Date> {
        Binding(get: {
            let d = CalendarDates.date(start ? form.start : form.end) ?? Date()
            return form.allDay && !start ? CalendarDates.adding(.day, -1, to: d) : d
        }, set: { value in
            let d = form.allDay ? CalendarDates.local.startOfDay(for: value) : value
            if start {
                form.start = CalendarDates.iso(d)
                if let end = CalendarDates.date(form.end), end <= d { form.end = CalendarDates.iso(CalendarDates.adding(form.allDay ? .day : .minute, form.allDay ? 1 : 30, to: d)) }
            } else { form.end = CalendarDates.iso(form.allDay ? CalendarDates.adding(.day, 1, to: d) : d) }
        })
    }
    private func searchOrAdd() {
        if query.contains("@") { add(CalendarPerson(address: query, name: "", optional: optional)) } else { send(.searchPeople(query: query)) }
    }
    private func add(_ person: CalendarPerson) {
        let address = person.address.trimmingCharacters(in: .whitespaces)
        guard address.contains("@"), !form.attendees.contains(where: { $0.address.caseInsensitiveCompare(address) == .orderedSame }) else { return }
        form.attendees.append(CalendarPerson(address: address, name: person.name, optional: optional))
        query = ""
        send(.searchPeople(query: ""))
    }
}

/// "Repeat": the web client's presets, or a custom rule, and when the series ends.
private struct RepeatField: View {
    @Binding var recurrence: RecurrenceDraft
    let start: Date
    var body: some View {
        Picker(L("ui.calendar_screen.repeat"), selection: $recurrence.`repeat`) {
            Text(L("ui.calendar_screen.does_not_repeat")).tag(Repeat.never)
            Text(L("ui.calendar_screen.every_weekday")).tag(Repeat.weekdays)
            Text(L("ui.calendar_screen.daily")).tag(Repeat.daily)
            Text(L("ui.calendar_screen.weekly_on", ["day": start.formatted(.dateTime.locale(L10n.locale).weekday(.wide))])).tag(Repeat.weekly)
            Text(L("ui.calendar_screen.monthly")).tag(Repeat.monthly)
            Text(L("ui.calendar_screen.yearly_on", ["date": start.formatted(.dateTime.locale(L10n.locale).day().month(.wide))])).tag(Repeat.yearly)
            Text(L("ui.calendar_screen.custom")).tag(Repeat.custom)
        }
        .onChange(of: recurrence.`repeat`) { _, value in
            if value == .custom && recurrence.interval == 0 {
                recurrence.interval = 1
                recurrence.unit = .week
                recurrence.weekdays = [UInt32(CalendarDates.local.component(.weekday, from: start) - 1)]
            }
        }
        if recurrence.`repeat` == .custom {
            HStack {
                Stepper(value: Binding(get: { Int(max(recurrence.interval, 1)) }, set: { recurrence.interval = UInt32($0) }), in: 1...99) {
                    Text(L("ui.calendar_screen.every_n", ["count": max(recurrence.interval, 1)]))
                }
                Picker("", selection: $recurrence.unit) {
                    Text(L("ui.calendar_screen.days")).tag(RepeatUnit.day)
                    Text(L("ui.calendar_screen.weeks")).tag(RepeatUnit.week)
                    Text(L("ui.calendar_screen.months")).tag(RepeatUnit.month)
                    Text(L("ui.calendar_screen.years")).tag(RepeatUnit.year)
                }
                .labelsHidden().frame(width: 120)
            }
            if recurrence.unit == .week {
                HStack(spacing: 4) {
                    ForEach(0..<7, id: \.self) { day in
                        let on = recurrence.weekdays.contains(UInt32(day))
                        Button(CalendarDates.local.veryShortWeekdaySymbols[day]) {
                            if on { recurrence.weekdays.removeAll { $0 == UInt32(day) } } else { recurrence.weekdays.append(UInt32(day)) }
                        }
                        .buttonStyle(GhostButtonStyle(accent: on))
                        .frame(width: 28, height: 28)
                        .background(on ? Theme.accentSoft : .clear, in: Circle())
                        .accessibilityAddTraits(on ? .isSelected : [])
                    }
                }
            }
            if recurrence.unit == .month || recurrence.unit == .year {
                Picker("", selection: $recurrence.byWeekday) {
                    Text(L("ui.calendar_screen.on_day", ["day": CalendarDates.local.component(.day, from: start)])).tag(false)
                    Text(L("ui.calendar_screen.on_the_same_weekday")).tag(true)
                }
                .pickerStyle(.radioGroup).labelsHidden()
            }
        }
        if recurrence.`repeat` != .never {
            Toggle(L("ui.calendar_screen.ends_on_a_date"), isOn: Binding(
                get: { !recurrence.endDate.isEmpty },
                set: { recurrence.endDate = $0 ? Self.day(CalendarDates.adding(.month, 3, to: start)) : "" }
            ))
            if !recurrence.endDate.isEmpty {
                DatePicker(L("ui.calendar_screen.end_of_series"), selection: Binding(
                    get: { Self.parse(recurrence.endDate) ?? start },
                    set: { recurrence.endDate = Self.day($0) }
                ), in: start..., displayedComponents: .date)
            }
        }
    }
    private static func formatter() -> DateFormatter {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.calendar = CalendarDates.local
        f.timeZone = CalendarDates.local.timeZone
        f.dateFormat = "yyyy-MM-dd"
        return f
    }
    static func day(_ date: Date) -> String { formatter().string(from: date) }
    static func parse(_ value: String) -> Date? { formatter().date(from: value) }
}
