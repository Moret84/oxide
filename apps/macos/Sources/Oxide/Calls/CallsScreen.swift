import SwiftUI
import OxideCore

private enum CallsTab: String, CaseIterable, Identifiable {
    case history, voicemail
    var id: Self { self }
    var title: String {
        switch self {
        case .history: L("ui.calls_screen.history")
        case .voicemail: L("ui.calls_screen.voicemail")
        }
    }
}

/// Call history and voicemail. Rows are grouped by local day, newest first.
struct CallsScreen: View {
    @Environment(Store.self) private var store
    @State private var tab: CallsTab = .history
    private var model: CallsView? { store.calls }
    private func send(_ action: CallsAction) { store.send(.calls(action: action)) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 14) {
                Text(L("ui.calls_screen.calls")).font(.figtree(22, .semibold))
                SegmentedPicker(selection: $tab, options: CallsTab.allCases, label: { tab in
                    let unread = model?.voicemails.filter { !$0.read }.count ?? 0
                    return tab == .voicemail && unread > 0 ? "\(tab.title) (\(unread))" : tab.title
                })
                Spacer()
                if model?.loadingHistory == true || model?.loadingVoicemails == true {
                    ProgressView().controlSize(.small)
                }
                IconButton(symbol: "arrow.clockwise", help: L("ui.calls_screen.refresh")) { send(.refresh) }
            }
            .padding(.horizontal, 24).padding(.vertical, 16)
            Divider()
            switch tab {
            case .history: history
            case .voicemail: voicemails
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Theme.surface)
    }

    private func banner(_ text: String) -> some View {
        ErrorBanner(text: text, retry: L("ui.calls_screen.try_again")) { send(.refresh) }
    }

    @ViewBuilder private var history: some View {
        CallPanel().padding(16)
        if let error = model?.error { banner(error) }
        let logs = model?.history ?? []
        if logs.isEmpty {
            empty(model?.loadingHistory == true ? L("ui.calls_screen.loading_history") : L("ui.calls_screen.no_recent_calls"),
                  symbol: "phone")
        } else {
            List {
                ForEach(CallsFormat.days(logs, date: \.startedAt), id: \.title) { day in
                    Section(day.title) {
                        ForEach(day.items, id: \.id) { log in CallLogRow(log: log) }
                    }
                }
                if model?.hasMore == true {
                    Button(L("ui.calls_screen.show_older_calls")) { send(.more) }
                        .buttonStyle(.link).disabled(model?.loadingHistory == true)
                }
            }
            .listStyle(.inset)
        }
    }

    @ViewBuilder private var voicemails: some View {
        if let error = model?.voicemailError { banner(error) }
        let list = model?.voicemails ?? []
        if list.isEmpty {
            empty(model?.loadingVoicemails == true ? L("ui.calls_screen.loading_voicemail") : L("ui.calls_screen.no_voicemail"),
                  symbol: "recordingtape")
        } else {
            List {
                ForEach(list, id: \.id) { voicemail in VoicemailRow(voicemail: voicemail) }
            }
            .listStyle(.inset)
        }
    }

    private func empty(_ text: String, symbol: String) -> some View {
        VStack(spacing: 10) {
            Image(systemName: symbol).font(.system(size: 30)).foregroundStyle(Theme.subtle)
            Muted(text, size: 14)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct CallLogRow: View {
    @Environment(Store.self) private var store
    let log: CallLogView
    private var canCall: Bool { log.canCallBack && !store.inCall }
    private func callBack(video: Bool) { store.send(.calls(action: .callBack(id: log.id, video: video))) }
    var body: some View {
        HStack(spacing: 12) {
            Avatar(name: log.name, id: log.peer.isEmpty ? log.id : log.peer, size: 36)
            VStack(alignment: .leading, spacing: 3) {
                Text(log.name).font(.figtree(14, .semibold))
                    .foregroundStyle(log.outcome == .missed ? Theme.danger : Theme.text)
                HStack(spacing: 5) {
                    Image(systemName: CallsFormat.symbol(log)).font(.system(size: 11))
                    Text(CallsFormat.summary(log))
                }
                .font(.figtree(12)).foregroundStyle(Theme.muted)
            }
            Spacer()
            Text(CallsFormat.time(log.startedAt)).font(.figtree(12)).foregroundStyle(Theme.subtle)
            if canCall {
                IconButton(symbol: "phone", help: L("ui.calls_screen.call_back"), side: 30, glyph: 14) { callBack(video: false) }
                IconButton(symbol: "video", help: L("ui.calls_screen.video_call_back"), side: 30, glyph: 14) { callBack(video: true) }
            }
        }
        .padding(.vertical, 4)
        .contextMenu {
            if canCall {
                Button(L("ui.calls_screen.call_back")) { callBack(video: false) }
                Button(L("ui.calls_screen.video_call_back")) { callBack(video: true) }
                Divider()
            }
            Button(L("ui.calls_screen.delete_from_history"), role: .destructive) {
                store.send(.calls(action: .deleteLog(id: log.id)))
            }
        }
    }
}

private struct VoicemailRow: View {
    @Environment(Store.self) private var store
    let voicemail: VoicemailView
    private var playing: Bool { store.voicemailPlayer.playing == voicemail.id }
    private var loading: Bool { store.calls?.audioLoading == voicemail.id }
    private func send(_ action: CallsAction) { store.send(.calls(action: action)) }
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Button {
                if playing { store.voicemailPlayer.stop() } else { send(.playVoicemail(id: voicemail.id)) }
            } label: {
                Group {
                    if loading { ProgressView().controlSize(.small) } else {
                        Image(systemName: playing ? "stop.fill" : "play.fill")
                    }
                }
                .frame(width: 34, height: 34)
                .background(Theme.accentSoft, in: Circle())
                .foregroundStyle(Theme.accentText)
            }
            .buttonStyle(.plain).pointerCursor()
            .accessibilityLabel(playing ? L("ui.calls_screen.stop") : L("ui.calls_screen.play"))
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    if !voicemail.read { Circle().fill(Theme.accent).frame(width: 7, height: 7) }
                    Text(voicemail.sender).font(.figtree(14, voicemail.read ? .regular : .semibold))
                    Spacer()
                    Text(CallsFormat.dateTime(voicemail.received)).font(.figtree(12)).foregroundStyle(Theme.subtle)
                }
                HStack(spacing: 8) {
                    if !voicemail.address.isEmpty && voicemail.address != voicemail.sender {
                        Muted(voicemail.address)
                    }
                    if let seconds = voicemail.durationSecs { Muted(CallsFormat.duration(Int64(seconds))) }
                }
                if let text = voicemail.transcription {
                    Text(text).font(.figtree(13)).foregroundStyle(Theme.textSoft)
                        .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .padding(.vertical, 6)
        .contextMenu {
            Button(voicemail.read ? L("ui.calls_screen.mark_as_unread") : L("ui.calls_screen.mark_as_read")) {
                send(.markVoicemail(id: voicemail.id, read: !voicemail.read))
            }
            Button(L("ui.calls_screen.delete"), role: .destructive) { send(.deleteVoicemail(id: voicemail.id)) }
        }
    }
}

/// Labels of the Calls screen, kept pure for tests.
enum CallsFormat {
    struct Day<Item> {
        let title: String
        let items: [Item]
    }

    static func days<Item>(_ items: [Item], date: (Item) -> String, now: Date = Date(),
                           calendar: Calendar = CalendarDates.local) -> [Day<Item>] {
        var order: [String] = []
        var groups: [String: [Item]] = [:]
        for item in items {
            let title = dayTitle(CalendarDates.date(date(item)), now: now, calendar: calendar)
            if groups[title] == nil { order.append(title) }
            groups[title, default: []].append(item)
        }
        return order.map { Day(title: $0, items: groups[$0] ?? []) }
    }

    static func dayTitle(_ date: Date?, now: Date = Date(), calendar: Calendar = CalendarDates.local) -> String {
        guard let date else { return L("ui.calls_screen.unknown_date") }
        if calendar.isDate(date, inSameDayAs: now) { return L("ui.calls_screen.today") }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
           calendar.isDate(date, inSameDayAs: yesterday) { return L("ui.calls_screen.yesterday") }
        return date.formatted(.dateTime.weekday(.wide).day().month(.wide).locale(L10n.locale))
    }

    static func duration(_ seconds: Int64) -> String {
        let h = seconds / 3600, m = (seconds % 3600) / 60, s = seconds % 60
        let pad = { (n: Int64) in String(format: "%02d", n) }
        if seconds < 60 { return L("ui.calls_screen.duration.seconds", ["s": seconds]) }
        if h > 0 { return L("ui.calls_screen.duration.hours", ["h": h, "m": pad(m)]) }
        return s == 0 ? L("ui.calls_screen.duration.minutes", ["m": m])
            : L("ui.calls_screen.duration.minutes_seconds", ["m": m, "s": pad(s)])
    }

    static func summary(_ log: CallLogView) -> String {
        let kind: String
        switch (log.outcome, log.direction) {
        case (.missed, _): kind = "missed"
        case (.declined, _): kind = "declined"
        case (_, .incoming): kind = "incoming"
        case (_, .outgoing): kind = "outgoing"
        case (_, .unknown): kind = "other"
        }
        let label = L("ui.calls_screen.summary.\(kind)\(log.group ? "_group" : "")")
        guard log.outcome != .missed, log.outcome != .declined, let secs = log.durationSecs else { return label }
        return L("ui.calls_screen.summary.with_duration", ["label": label, "duration": duration(secs)])
    }

    static func symbol(_ log: CallLogView) -> String {
        if log.outcome == .missed || log.outcome == .declined { return "phone.down" }
        switch log.direction {
        case .incoming: return "phone.arrow.down.left"
        case .outgoing: return "phone.arrow.up.right"
        case .unknown: return "phone"
        }
    }

    static func time(_ iso: String) -> String {
        CalendarDates.date(iso)?.formatted(date: .omitted, time: .shortened) ?? ""
    }

    static func dateTime(_ iso: String) -> String {
        guard let date = CalendarDates.date(iso) else { return "" }
        return "\(dayTitle(date)) · \(date.formatted(date: .omitted, time: .shortened))"
    }
}
