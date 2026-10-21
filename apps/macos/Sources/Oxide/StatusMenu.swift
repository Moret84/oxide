import SwiftUI
import OxideCore

/// The top bar's account button (photo and availability), opening the status card like the web client's "Me".
struct AccountButton: View {
    @Environment(Store.self) private var store
    let name: String
    let id: String
    let email: String
    @State private var open = false
    var body: some View {
        Button { open.toggle() } label: {
            Avatar(name: name, id: id, size: 32, presence: store.me?.presence)
                .frame(width: 36, height: 36)
        }
        .buttonStyle(GhostButtonStyle())
        .help(L("ui.main_view.your_account"))
        .accessibilityLabel(L("ui.main_view.your_account"))
        .popover(isPresented: $open, arrowEdge: .bottom) {
            StatusCard(name: name, id: id, email: email, close: { open = false })
        }
    }
}

private struct StatusCard: View {
    @Environment(Store.self) private var store
    let name: String
    let id: String
    let email: String
    let close: () -> Void
    @State private var duration = StatusDuration.never
    @State private var editingNote = false
    var body: some View {
        let me = store.me ?? MeView(presence: nil, forced: false, forcedUntil: nil, note: "", noteUntil: nil,
                                    outOfOffice: false, outOfOfficeNote: "", noteLimit: 2000, saving: false, error: nil,
                                    outOfOfficeSettings: OutOfOfficeView(settings: nil, loading: false, saving: false, saved: 0))
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Avatar(name: name, id: id, size: 44, presence: me.presence)
                VStack(alignment: .leading, spacing: 3) {
                    Text(name).font(.figtree(15, .semibold))
                    if !email.isEmpty { Muted(email, size: 12) }
                    if let presence = me.presence {
                        Text(PresenceBadge.label(presence) + (me.forcedUntil.map { " · \($0)" } ?? ""))
                            .font(.figtree(12)).foregroundStyle(Theme.textSoft)
                    }
                }
                Spacer(minLength: 0)
                if me.saving { ProgressView().controlSize(.small) }
            }
            if me.outOfOffice {
                Label(me.outOfOfficeNote.isEmpty ? L("ui.status_menu.automatic_replies_on") : me.outOfOfficeNote,
                      systemImage: "airplane")
                    .font(.figtree(12)).foregroundStyle(Theme.textSoft)
                    .lineLimit(3)
            }
            Divider()
            noteSection(me)
            Divider()
            VStack(alignment: .leading, spacing: 2) {
                ForEach(MyStatus.allCases, id: \.self) { status in
                    Button { store.send(.me(action: .setStatus(status: status, duration: duration))) } label: {
                        HStack(spacing: 10) {
                            PresenceBadge(presence: PresenceView(status: status.presence, outOfOffice: false), size: 12)
                            Text(status.label).font(.figtree(13))
                            Spacer()
                            if me.presence?.status == status.presence && me.forced {
                                Image(systemName: "checkmark").font(.system(size: 11)).foregroundStyle(Theme.accent)
                            }
                        }
                        .padding(.horizontal, 8).frame(minHeight: 28).contentShape(Rectangle())
                    }
                    .buttonStyle(GhostButtonStyle())
                }
                HStack {
                    Text(L("ui.status_menu.reset_after")).font(.figtree(12)).foregroundStyle(Theme.muted)
                    Picker("", selection: $duration) {
                        ForEach(StatusDuration.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    .labelsHidden().frame(maxWidth: 170)
                }
                .padding(.top, 4)
                if me.forced {
                    Button(L("ui.status_menu.reset_status")) { store.send(.me(action: .resetStatus)) }
                        .buttonStyle(OutlineButtonStyle()).padding(.top, 4)
                }
            }
            Divider()
            Button {
                close()
                store.openSettings(.outOfOffice)
            } label: {
                Label(L("ui.status_menu.schedule_out_of_office"), systemImage: "airplane.departure")
                    .font(.figtree(13)).frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
            }
            .buttonStyle(.plain).pointerCursor()
            Button {
                close()
                store.send(.navigate(nav: .settings))
            } label: {
                Label(L("ui.status_menu.settings"), systemImage: "gearshape")
                    .font(.figtree(13)).frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
            }
            .buttonStyle(.plain).pointerCursor()
            if let error = me.error {
                HStack(alignment: .top) {
                    Text(error).font(.figtree(12)).foregroundStyle(Theme.errorText)
                    Spacer()
                    IconButton(symbol: "xmark", help: L("ui.status_menu.close"), side: 20, glyph: 10) {
                        store.send(.me(action: .dismissError))
                    }
                }
            }
        }
        .padding(16)
        .frame(width: 320)
        .onAppear { store.send(.me(action: .refresh)) }
        .sheet(isPresented: $editingNote) { StatusNoteSheet(me: me) }
    }

    @ViewBuilder private func noteSection(_ me: MeView) -> some View {
        Button { editingNote = true } label: {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "text.bubble").foregroundStyle(Theme.accent)
                VStack(alignment: .leading, spacing: 2) {
                    Text(me.note.isEmpty ? L("ui.status_menu.set_status_message") : me.note)
                        .font(.figtree(13)).foregroundStyle(Theme.text).lineLimit(4)
                        .multilineTextAlignment(.leading)
                    if let until = me.noteUntil { Muted(until, size: 11) }
                }
                Spacer(minLength: 0)
                Image(systemName: "pencil").foregroundStyle(Theme.subtle)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain).pointerCursor()
    }
}

/// Writes or clears the status message, as the web client's "Set status message" dialog.
private struct StatusNoteSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let me: MeView
    @State private var text = ""
    @State private var duration = StatusDuration.never
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(L("ui.status_menu.set_status_message")).font(.figtree(17, .semibold))
            TextEditor(text: $text)
                .font(.figtree(13))
                .frame(height: 90)
                .padding(4)
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.borderStrong))
                .onChange(of: text) { _, value in
                    if value.count > Int(me.noteLimit) { text = String(value.prefix(Int(me.noteLimit))) }
                }
                .accessibilityLabel(L("ui.status_menu.status_message"))
            HStack {
                Muted(L("ui.status_menu.clear_after"), size: 12)
                Picker("", selection: $duration) {
                    ForEach(StatusDuration.allCases, id: \.self) { Text($0.label).tag($0) }
                }
                .labelsHidden().frame(maxWidth: 180)
                Spacer()
                Muted("\(text.count)/\(me.noteLimit)", size: 11)
            }
            HStack {
                if !me.note.isEmpty {
                    Button(L("ui.status_menu.clear_message")) {
                        store.send(.me(action: .clearNote))
                        dismiss()
                    }
                    .buttonStyle(OutlineButtonStyle())
                }
                Spacer()
                Button(L("ui.status_menu.cancel")) { dismiss() }.buttonStyle(OutlineButtonStyle())
                    .keyboardShortcut(.cancelAction)
                Button(L("ui.status_menu.done")) {
                    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
                    store.send(.me(action: trimmed.isEmpty ? .clearNote : .setNote(text: trimmed, duration: duration)))
                    dismiss()
                }
                .buttonStyle(PrimaryButtonStyle())
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 420)
        .onAppear { text = me.note }
    }
}

extension MyStatus: CaseIterable {
    public static var allCases: [MyStatus] { [.available, .busy, .doNotDisturb, .beRightBack, .away, .offline] }
    var presence: PresenceStatus {
        switch self {
        case .available: .available
        case .busy: .busy
        case .doNotDisturb: .doNotDisturb
        case .beRightBack: .beRightBack
        case .away: .away
        case .offline: .offline
        }
    }
    var label: String {
        switch self {
        case .available: L("ui.presence.available")
        case .busy: L("ui.presence.busy")
        case .doNotDisturb: L("ui.presence.do_not_disturb")
        case .beRightBack: L("ui.presence.be_right_back")
        case .away: L("ui.status_menu.appear_away")
        case .offline: L("ui.status_menu.appear_offline")
        }
    }
}

extension StatusDuration: CaseIterable {
    public static var allCases: [StatusDuration] {
        [.never, .halfHour, .oneHour, .twoHours, .fourHours, .today, .thisWeek]
    }
    var label: String {
        switch self {
        case .halfHour: L("ui.status_menu.30_minutes")
        case .oneHour: L("ui.status_menu.1_hour")
        case .twoHours: L("ui.status_menu.2_hours")
        case .fourHours: L("ui.status_menu.4_hours")
        case .today: L("ui.status_menu.today")
        case .thisWeek: L("ui.status_menu.this_week")
        case .never: L("ui.status_menu.never")
        }
    }
}
