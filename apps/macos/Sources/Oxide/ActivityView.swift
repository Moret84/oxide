import SwiftUI
import OxideCore

struct ActivityList: View {
    @Environment(Store.self) private var store
    let unreadOnly: Bool
    let sidebar: Sidebar
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 4) {
                ForEach(sidebar.activities, id: \.id) { row in
                    ActivityRowView(row: row)
                }
                if sidebar.activities.isEmpty && sidebar.activityError == nil {
                    Muted(sidebar.activityLoading ? L("ui.activity_view.loading_notifications")
                          : unreadOnly ? L("ui.activity_view.no_unread_notifications")
                          : L("ui.activity_view.no_notifications_to_show"))
                        .padding(16)
                }
                if let problem = sidebar.activityError {
                    Muted(problem).padding(12)
                    Button(L("ui.activity_view.try_again")) { store.send(.refresh) }.buttonStyle(GhostButtonStyle())
                }
                if sidebar.moreActivity {
                    Button(sidebar.activityLoading ? L("ui.activity_view.loading") : L("ui.activity_view.earlier_notifications")) { store.send(.moreActivity) }
                        .buttonStyle(GhostButtonStyle()).font(.figtree(12)).padding(8)
                        .disabled(sidebar.activityLoading)
                }
            }
            .padding(8)
        }
    }
}

/// One feed item as Teams draws it, with a missed call's actions under it.
private struct ActivityRowView: View {
    @Environment(Store.self) private var store
    let row: ActivityRow
    @State private var hovered = false

    /// The call actions sit outside the row's button, so each is its own button for clicks and VoiceOver.
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button { store.send(.openActivity(id: row.id)) } label: {
                HStack(alignment: .top, spacing: 10) {
                    Avatar(name: row.actor, id: row.actorId, size: 36)
                        .overlay(alignment: .bottomTrailing) { ActivityBadgeView(badge: row.badge, reaction: row.reaction).offset(x: 4, y: 4) }
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text(row.title)
                                .font(.figtree(13, row.unread ? .semibold : .medium))
                                .foregroundStyle(Theme.text).lineLimit(2)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            Text(row.shortTime).font(.figtree(11)).foregroundStyle(Theme.subtle)
                        }
                        if !row.context.isEmpty {
                            Text(row.context).font(.figtree(12)).foregroundStyle(Theme.textSoft).lineLimit(1)
                        }
                        if !row.preview.isEmpty {
                            Text(row.preview).font(.figtree(12)).foregroundStyle(Theme.muted).lineLimit(1)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if row.unread {
                        Circle().fill(Theme.accent).frame(width: 8, height: 8).padding(.top, 5).accessibilityLabel(L("ui.activity_view.unread"))
                    }
                }
                .padding(EdgeInsets(top: 10, leading: 10, bottom: row.callBack == nil ? 10 : 4, trailing: 10))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain).pointerCursor()
            .accessibilityAddTraits(row.selected ? .isSelected : [])
            if let mri = row.callBack {
                HStack(spacing: 8) {
                    Button(L("ui.activity_view.call")) { store.send(.call(action: .start(mri: mri, video: false))) }
                        .buttonStyle(OutlineButtonStyle())
                    if let chat = row.chatId {
                        Button(L("ui.activity_view.chat")) { store.openChat(chat) }
                        .buttonStyle(OutlineButtonStyle())
                    }
                }
                .font(.figtree(12, .semibold))
                // Lined up with the text, past the avatar.
                .padding(EdgeInsets(top: 0, leading: 56, bottom: 10, trailing: 10))
            }
        }
        .background(row.selected ? Theme.accentSoft : hovered ? Theme.input : .clear, in: RoundedRectangle(cornerRadius: 8))
        .onHover { hovered = $0 }
    }
}

/// Small chip on the avatar: what kind of activity this is.
struct ActivityBadgeView: View {
    let badge: ActivityBadge
    let reaction: String?

    var body: some View {
        if let symbol = Self.symbol(badge) {
            Group {
                if badge == .reaction, let reaction {
                    Text(reaction).font(.system(size: 10))
                } else {
                    Image(systemName: symbol).font(.system(size: 8, weight: .bold)).foregroundStyle(.white)
                }
            }
            .frame(width: 18, height: 18)
            .background(Self.color(badge), in: Circle())
            .overlay(Circle().strokeBorder(Theme.surface, lineWidth: 2))
            .accessibilityHidden(true)
        }
    }

    static func symbol(_ badge: ActivityBadge) -> String? {
        switch badge {
        case .none: nil
        case .mention: "at"
        case .reply: "arrowshape.turn.up.left.fill"
        case .reaction: "hand.thumbsup.fill"
        case .missedCall: "phone.arrow.down.left.fill"
        case .voicemail: "recordingtape"
        case .meeting: "calendar"
        case .app: "app.fill"
        case .team: "person.3.fill"
        case .urgent: "exclamationmark"
        }
    }
    static func color(_ badge: ActivityBadge) -> Color {
        switch badge {
        case .mention: .orange
        case .missedCall, .urgent: Theme.danger
        case .reaction: Theme.input
        default: Theme.accent
        }
    }
}

struct ActivityOverview: View {
    let row: ActivityRow?
    var body: some View {
        if let row, row.meeting {
            MeetingDetailPane(row: row)
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(row?.title ?? L("ui.activity_view.activity")).font(.figtree(24, .semibold))
                    if let row {
                        if !row.context.isEmpty { Text(row.context).font(.figtree(16, .medium)) }
                        Muted(row.time)
                        Text(row.preview).font(.figtree(15)).textSelection(.enabled)
                    } else {
                        Muted(L("ui.activity_view.select_a_notification_to_open_it"), size: 14)
                    }
                }
                .frame(maxWidth: 700, alignment: .leading)
                .padding(32)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

/// Teams' "Détails" of a meeting notification: the event, its answer, and its attendees.
private struct MeetingDetailPane: View {
    @Environment(Store.self) private var store
    let row: ActivityRow

    /// The selected calendar event, once it is this notification's.
    private var event: CalendarEventView? {
        guard let id = row.eventId else { return nil }
        return store.calendar?.selected.flatMap { $0.id == id ? $0 : nil }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    header
                    if let event {
                        details(event)
                    } else if store.calendar?.loadingDetail == true {
                        ProgressView().frame(maxWidth: .infinity)
                    } else {
                        if !row.context.isEmpty { Label(row.context, systemImage: "calendar") }
                        Muted(store.calendar?.error ?? L("ui.activity_view.meeting_details_aren_t_available"), size: 13)
                    }
                }
                .frame(maxWidth: 720, alignment: .leading)
                .padding(28)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let event {
                Divider().overlay(Theme.border)
                attendees(event).frame(width: 280)
            }
        }
    }

    private var header: some View {
        HStack(spacing: 12) {
            Image(systemName: "calendar")
                .font(.system(size: 16, weight: .semibold)).foregroundStyle(.white)
                .frame(width: 34, height: 34)
                .background(Theme.accent, in: RoundedRectangle(cornerRadius: 8))
            Text(event?.form.subject.isEmpty == false ? event!.form.subject : row.preview)
                .font(.figtree(20, .semibold)).lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let chat = row.chatId {
                Button(L("ui.activity_view.chat")) { store.openChat(chat) }
                .buttonStyle(OutlineButtonStyle())
            }
            if let event, !event.joinUrl.isEmpty, !event.cancelled {
                Button(L("ui.activity_view.join")) { join(event) }.buttonStyle(PrimaryButtonStyle())
            }
        }
    }

    @ViewBuilder private func details(_ event: CalendarEventView) -> some View {
        if event.cancelled {
            Label(L("ui.activity_view.this_meeting_has_been_canceled"), systemImage: "xmark.circle")
                .font(.figtree(13, .semibold)).foregroundStyle(Theme.danger)
        }
        if event.canRespond {
            HStack(spacing: 8) {
                Menu {
                    Button(L("ui.activity_view.accept")) { respond("Accept") }
                    Button(L("ui.activity_view.tentative")) { respond("TentativelyAccept") }
                    Button(L("ui.activity_view.decline")) { respond("Decline") }
                } label: {
                    Label(L("ui.activity_view.rsvp"), systemImage: "arrowshape.turn.up.left")
                }
                .fixedSize()
                .disabled(store.calendar?.busy == true)
                if !event.response.isEmpty {
                    Muted(L("ui.activity_view.your_response") + Self.responseLabel(event.response), size: 12)
                }
            }
        }
        VStack(alignment: .leading, spacing: 12) {
            Label(row.context.isEmpty ? Self.range(event) : row.context, systemImage: "clock")
            if !event.form.location.isEmpty { Label(event.form.location, systemImage: "mappin.and.ellipse") }
            if event.recurring { Label(L("ui.activity_view.recurring_meeting"), systemImage: "repeat") }
        }
        .font(.figtree(14))
        if !event.form.description.isEmpty {
            Divider()
            Text(event.form.description).font(.figtree(13)).textSelection(.enabled)
        }
        Button(L("ui.activity_view.open_in_calendar")) { store.send(.navigate(nav: .calendar)) }
            .buttonStyle(.link)
    }

    private func attendees(_ event: CalendarEventView) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text(L("ui.activity_view.tracking")).font(.figtree(15, .semibold))
                if !event.organizer.isEmpty {
                    person(name: event.organizer, id: event.organizer, status: L("ui.activity_view.organizer"), color: Theme.muted)
                    Divider()
                }
                ForEach(Array(event.form.attendees.enumerated()), id: \.offset) { _, attendee in
                    let name = attendee.name.isEmpty ? attendee.address : attendee.name
                    let (label, color) = Self.attendeeStatus(attendee.response)
                    person(name: name, id: attendee.address, status: label + (attendee.optional ? L("ui.activity_view.optional") : ""), color: color)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func person(name: String, id: String, status: String, color: Color) -> some View {
        HStack(spacing: 10) {
            Avatar(name: name, id: id, size: 30)
            VStack(alignment: .leading, spacing: 1) {
                Text(name).font(.figtree(13)).lineLimit(1)
                Text(status).font(.figtree(11)).foregroundStyle(color)
            }
        }
    }

    private func join(_ event: CalendarEventView) {
        store.send(.call(action: .prepareMeeting(url: event.joinUrl, title: event.title)))
    }

    private func respond(_ response: String) {
        store.send(.calendar(action: .respond(response: response, series: false)))
    }

    static func range(_ event: CalendarEventView) -> String {
        guard let start = CalendarDates.date(event.form.start) else { return "" }
        let end = CalendarDates.date(event.form.end)
        let day = start.formatted(.dateTime.day().month(.abbreviated).locale(L10n.locale))
        if event.form.allDay { return day }
        let time = { (date: Date) in date.formatted(date: .omitted, time: .shortened) }
        return end.map { "\(day), \(time(start)) - \(time($0))" } ?? "\(day), \(time(start))"
    }

    static func responseLabel(_ value: String) -> String {
        let key = ["Accepted": "accepted", "Declined": "declined", "Tentative": "tentative",
                   "TentativelyAccepted": "tentative", "Organizer": "organizer", "NotResponded": "none",
                   "None": "none"][value]
        return key.map { L("ui.activity_view.response.\($0)") } ?? value
    }

    static func attendeeStatus(_ response: String) -> (String, Color) {
        switch response {
        case "Accepted": (L("ui.activity_view.accepted"), .green)
        case "Declined": (L("ui.activity_view.declined"), Theme.danger)
        case "Tentative", "TentativelyAccepted": (L("ui.activity_view.tentative"), .orange)
        case "Organizer": (L("ui.activity_view.organizer"), Theme.muted)
        default: (L("ui.activity_view.no_response"), Theme.muted)
        }
    }
}

private extension Store {
    func openChat(_ id: String) {
        send(.navigate(nav: .chat))
        send(.select(target: Target(team: nil, id: id)))
    }
}
