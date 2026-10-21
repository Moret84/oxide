import SwiftUI
import OxideCore

/// The selected event beside the calendar: answers, edit, cancel or delete, and the attendees.
struct CalendarEventDetails: View {
    @Environment(Store.self) private var store
    let event: CalendarEventView
    /// For one date of a series: whether edits and answers go to the whole series.
    @Binding var wholeSeries: Bool
    /// "cancel" or "delete", confirmed by the calendar screen.
    @Binding var destructive: String?
    let join: (CalendarEventView) -> Void
    private var model: CalendarView? { store.calendar }
    private var locked: Bool { model?.busy == true || model?.uncertain == true }
    private func send(_ action: CalendarAction) { store.send(.calendar(action: action)) }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack { Text(L("ui.calendar_screen.details")).font(.figtree(15, .semibold)); Spacer(); Button { send(.closeDetail) } label: { Image(systemName: "xmark") }.disabled(model?.busy == true) }
                Text(event.title).font(.figtree(21, .semibold)).textSelection(.enabled)
                if event.cancelled { Text(L("ui.calendar_screen.meeting_canceled")).foregroundStyle(Theme.danger) }
                if let start = CalendarDates.date(event.form.start), let end = CalendarDates.date(event.form.end) {
                    Text(when(start: start, end: end, allDay: event.form.allDay)).font(.figtree(13)).foregroundStyle(Theme.textSoft)
                }
                Text(L("ui.calendar_screen.time_zone", ["identifier": TimeZone.current.identifier])).font(.figtree(11)).foregroundStyle(Theme.muted)
                if !event.organizer.isEmpty { Label(event.organizer, systemImage: "person") }
                if !event.form.location.isEmpty { Label(event.form.location, systemImage: "mappin") }
                if !event.joinUrl.isEmpty && !event.cancelled {
                    Button(L("ui.calendar_screen.join_meeting"), systemImage: "video") { join(event) }.buttonStyle(PrimaryButtonStyle())
                    Button(L("ui.calendar_screen.open_in_teams_on_the_web")) { store.send(.openUrl(url: event.joinUrl)) }
                        .buttonStyle(.link)
                }
                if !event.recurrenceText.isEmpty {
                    Label(event.recurrenceText, systemImage: "repeat").font(.figtree(12)).foregroundStyle(Theme.textSoft)
                }
                if event.occurrence && (event.canEdit || event.canRespond || event.canCancel || event.canDelete) {
                    SegmentedPicker(selection: $wholeSeries, options: [false, true]) {
                        $0 ? L("ui.calendar_screen.the_whole_series") : L("ui.calendar_screen.this_event")
                    }
                }
                if !event.response.isEmpty { Text(L("ui.calendar_screen.your_response", ["response": responseLabel(event.response)])).font(.figtree(12)) }
                if event.canRespond {
                    let status = CalendarStatus(event)
                    HStack(spacing: 0) {
                        responseButton(L("ui.calendar_screen.accept"), systemImage: "checkmark", value: "Accept", current: status == .accepted)
                        Divider().frame(height: 18)
                        responseButton(L("ui.calendar_screen.tentative"), systemImage: "questionmark", value: "TentativelyAccept", current: status == .tentative)
                        Divider().frame(height: 18)
                        responseButton(L("ui.calendar_screen.decline"), systemImage: "xmark", value: "Decline", current: status == .declined)
                    }
                    .padding(2)
                    .background(Theme.input, in: RoundedRectangle(cornerRadius: 9))
                    .disabled(locked)
                }
                if event.canEdit {
                    Button(L("ui.calendar_screen.edit")) { send(.edit(series: event.occurrence && wholeSeries)) }.disabled(locked)
                }
                if event.canCancel {
                    Button(L("ui.calendar_screen.cancel_meeting"), role: .destructive) { destructive = "cancel" }.disabled(locked)
                }
                if event.canDelete { Button(L("ui.calendar_screen.delete_from_my_calendar"), role: .destructive) { destructive = "delete" }.disabled(locked) }
                if model?.busy == true { ProgressView().controlSize(.small) }
                if !event.form.description.isEmpty { Divider(); Text(event.form.description).font(.figtree(13)).textSelection(.enabled) }
                if !event.form.attendees.isEmpty {
                    Divider(); Text(L("ui.calendar_screen.attendees")).font(.figtree(13, .semibold))
                    ForEach(Array(event.form.attendees.enumerated()), id: \.offset) { _, person in attendeeRow(person) }
                }
            }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
        }
    }
    private func when(start: Date, end: Date, allDay: Bool) -> String {
        let c = CalendarDates.local
        let long = Date.FormatStyle.dateTime.locale(L10n.locale).weekday(.wide).day().month(.wide)
        if allDay {
            let last = CalendarDates.adding(.day, -1, to: end)
            return c.isDate(start, inSameDayAs: last) ? start.formatted(long) + L("ui.calendar_screen.all_day") : start.formatted(long) + " → " + last.formatted(long)
        }
        if c.isDate(start, inSameDayAs: end) {
            return start.formatted(long) + " · " + CalendarText.time(start) + " – " + CalendarText.time(end)
        }
        return start.formatted(Date.FormatStyle(date: .abbreviated, time: .shortened, locale: L10n.locale)) + " → " + end.formatted(Date.FormatStyle(date: .abbreviated, time: .shortened, locale: L10n.locale))
    }
    /// The current answer is the filled one.
    private func responseButton(_ title: String, systemImage: String, value: String, current: Bool) -> some View {
        Button { send(.respond(response: value, series: wholeSeries)) } label: {
            Label(title, systemImage: systemImage)
                .font(.figtree(12, current ? .semibold : .regular))
                .lineLimit(1)
                .fixedSize()
                .foregroundStyle(current ? Theme.accentText : Theme.textSoft)
                .frame(maxWidth: .infinity, minHeight: 28)
                .background(current ? Theme.surface : Color.clear, in: RoundedRectangle(cornerRadius: 7))
                .shadow(color: .black.opacity(current ? 0.08 : 0), radius: 1, y: 1)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .pointerCursor()
        .accessibilityAddTraits(current ? .isSelected : [])
    }
    private func attendeeRow(_ person: CalendarPerson) -> some View {
        let answer = [
            "Accepted": L("ui.calendar_screen.accepted"), "TentativelyAccepted": L("ui.calendar_screen.tentative"),
            "Tentative": L("ui.calendar_screen.tentative"), "Declined": L("ui.calendar_screen.declined"),
            "Organizer": L("ui.calendar_screen.organizer"),
        ][person.response] ?? ""
        let info = [person.optional ? L("ui.calendar_screen.optional") : "", answer].filter { !$0.isEmpty }.joined(separator: " · ")
        return HStack(spacing: 10) {
            AttendeeBadge(person: person)
            VStack(alignment: .leading, spacing: 1) {
                Text(person.name.isEmpty ? person.address : person.name).font(.figtree(13)).lineLimit(1)
                Text(info.isEmpty ? person.address : info).font(.figtree(11)).foregroundStyle(Theme.muted).lineLimit(1)
            }
        }.help(person.address)
    }
    private func responseLabel(_ value: String) -> String {
        [
            "Accepted": L("ui.calendar_screen.accepted_2"), "Declined": L("ui.calendar_screen.declined_2"),
            "Tentative": L("ui.calendar_screen.tentative"), "Organizer": L("ui.calendar_screen.organizer"),
            "NotResponded": L("ui.calendar_screen.not_responded"), "None": L("ui.calendar_screen.none"),
        ][value] ?? value
    }
}

extension Bool: @retroactive Identifiable { public var id: Bool { self } }
