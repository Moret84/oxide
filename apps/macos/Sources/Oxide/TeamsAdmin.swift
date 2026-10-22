import SwiftUI
import OxideCore

extension TeamRow: Identifiable {}

/// "+" of the Teams list: create a team, or browse the gallery and join one.
struct TeamsAddMenu: View {
    @Environment(Store.self) private var store
    @State private var creating = false
    var body: some View {
        Menu {
            Button(L("ui.teams_admin.create_team")) { creating = true }
            Button(L("ui.teams_admin.join_or_create")) { store.send(.teamsAdmin(action: .openGallery)) }
        } label: {
            Image(systemName: "plus").font(.system(size: 15)).frame(width: 34, height: 34)
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        .help(L("ui.teams_admin.join_or_create"))
        .accessibilityLabel(L("ui.teams_admin.join_or_create"))
        .sheet(isPresented: $creating) { NewTeamSheet() }
        .sheet(isPresented: Binding(
            get: { store.teamsAdmin?.gallery != nil },
            set: { if !$0 { store.send(.teamsAdmin(action: .closeGallery)) } }
        )) {
            if let gallery = store.teamsAdmin?.gallery {
                TeamsGallerySheet(gallery: gallery) {
                    store.send(.teamsAdmin(action: .closeGallery))
                    creating = true
                }
            }
        }
    }
}

/// Error of the team dialogs, with a close button.
private struct AdminError: View {
    @Environment(Store.self) private var store
    var body: some View {
        if let error = store.teamsAdmin?.error {
            HStack(alignment: .top) {
                Text(error).font(.figtree(12)).foregroundStyle(Theme.errorText).textSelection(.enabled)
                Spacer()
                IconButton(symbol: "xmark", help: L("ui.status_menu.close"), side: 20, glyph: 10) {
                    store.send(.teamsAdmin(action: .dismissError))
                }
            }
        }
    }
}

/// Closes a creation dialog once the core counts one more creation.
private struct CloseOnCreated: ViewModifier {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var start: UInt32?
    func body(content: Content) -> some View {
        content
            .onAppear {
                start = store.teamsAdmin?.created ?? 0
                store.send(.teamsAdmin(action: .dismissError))
            }
            .onChange(of: store.teamsAdmin?.created) { _, value in
                if let value, let start, value > start { dismiss() }
            }
    }
}

/// Cancel and Create of a creation dialog; Create needs a name.
private struct CreateButtons: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let name: String
    let create: () -> Void
    var body: some View {
        let creating = store.teamsAdmin?.creating == true
        HStack {
            Spacer()
            Button(L("ui.chat_admin.cancel")) { dismiss() }.buttonStyle(OutlineButtonStyle()).keyboardShortcut(.cancelAction)
            Button(creating ? L("ui.teams_admin.creating") : L("ui.teams_admin.create"), action: create)
                .buttonStyle(PrimaryButtonStyle())
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || creating)
                .keyboardShortcut(.defaultAction)
        }
    }
}

struct NewTeamSheet: View {
    @Environment(Store.self) private var store
    @State private var name = ""
    @State private var description = ""
    @State private var visibility = NewTeamVisibility.private
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(L("ui.teams_admin.create_team")).font(.figtree(17, .semibold))
            TextField(L("ui.teams_admin.team_name"), text: $name).textFieldStyle(.roundedBorder)
            TextField(L("ui.teams_admin.description_optional"), text: $description, axis: .vertical)
                .textFieldStyle(.roundedBorder).lineLimit(3...5)
            Picker(L("ui.teams_admin.privacy"), selection: $visibility) {
                Text(L("ui.teams_admin.private")).tag(NewTeamVisibility.private)
                Text(L("ui.teams_admin.public")).tag(NewTeamVisibility.public)
                Text(L("ui.teams_admin.org_wide")).tag(NewTeamVisibility.orgWide)
            }
            .pickerStyle(.radioGroup)
            Muted(visibility == .private ? L("ui.teams_admin.private_explanation")
                  : visibility == .public ? L("ui.teams_admin.public_explanation")
                  : L("ui.teams_admin.org_wide_explanation"), size: 12)
            AdminError()
            CreateButtons(name: name) {
                store.send(.teamsAdmin(action: .createTeam(name: name, description: description, visibility: visibility)))
            }
        }
        .padding(20)
        .frame(width: 440)
        .modifier(CloseOnCreated())
    }
}

struct NewChannelSheet: View {
    @Environment(Store.self) private var store
    let team: TeamRow
    @State private var name = ""
    @State private var description = ""
    @State private var priv = false
    @State private var showForEveryone = false
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(L("ui.teams_admin.new_channel_in", ["team": team.name])).font(.figtree(17, .semibold))
            TextField(L("ui.teams_admin.channel_name"), text: $name).textFieldStyle(.roundedBorder)
            TextField(L("ui.teams_admin.description_optional"), text: $description, axis: .vertical)
                .textFieldStyle(.roundedBorder).lineLimit(2...4)
            Picker(L("ui.teams_admin.channel_type"), selection: $priv) {
                Text(L("ui.teams_admin.standard_channel")).tag(false)
                Text(L("ui.teams_admin.private_channel")).tag(true)
            }
            .pickerStyle(.radioGroup)
            if !priv {
                Toggle(L("ui.teams_admin.show_for_everyone"), isOn: $showForEveryone)
            }
            AdminError()
            CreateButtons(name: name) {
                store.send(.teamsAdmin(action: .createChannel(teamId: team.id, name: name, description: description,
                                                              private: priv, showForEveryone: showForEveryone)))
            }
        }
        .padding(20)
        .frame(width: 440)
        .modifier(CloseOnCreated())
    }
}

/// "Join or create a team": a code field, then suggested or searched teams.
struct TeamsGallerySheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let gallery: GalleryView
    let create: () -> Void
    @State private var query = ""
    @State private var code = ""
    private let columns = [GridItem(.adaptive(minimum: 200), spacing: 12)]
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(L("ui.teams_admin.join_or_create")).font(.figtree(18, .semibold))
                Spacer()
                FileSearchField(placeholder: L("ui.teams_admin.search_teams"), current: query, delay: .milliseconds(400)) {
                    query = $0
                    store.send(.teamsAdmin(action: .gallerySearch(query: $0)))
                }
                .frame(width: 240)
            }
            HStack(spacing: 12) {
                Button(action: create) {
                    Label(L("ui.teams_admin.create_team"), systemImage: "plus")
                }
                .buttonStyle(PrimaryButtonStyle())
                Divider().frame(height: 24)
                TextField(L("ui.teams_admin.enter_code"), text: $code)
                    .textFieldStyle(.roundedBorder).frame(width: 180)
                    .onSubmit { store.send(.teamsAdmin(action: .joinWithCode(code: code))) }
                Button(gallery.joiningCode ? L("ui.teams_admin.joining") : L("ui.teams_admin.join_with_code")) {
                    store.send(.teamsAdmin(action: .joinWithCode(code: code)))
                }
                .buttonStyle(OutlineButtonStyle())
                .disabled(code.trimmingCharacters(in: .whitespaces).isEmpty || gallery.joiningCode)
            }
            if gallery.codePending { Muted(L("ui.teams_admin.code_pending"), size: 12) }
            AdminError()
            ScrollView {
                LazyVGrid(columns: columns, alignment: .leading, spacing: 12) {
                    ForEach(gallery.teams, id: \.groupId) { team in card(team) }
                }
                if gallery.teams.isEmpty && !gallery.loading {
                    Muted(gallery.query.isEmpty ? L("ui.teams_admin.no_suggestions") : L("ui.teams_admin.no_teams_found"), size: 13)
                        .padding(.vertical, 20)
                }
                if gallery.loading { ProgressView().padding() }
                if gallery.hasMore && !gallery.loading {
                    Button(L("ui.teams_admin.more_teams")) { store.send(.teamsAdmin(action: .galleryMore)) }
                        .buttonStyle(GhostButtonStyle(accent: true)).padding(.vertical, 8)
                }
            }
            HStack {
                Spacer()
                Button(L("ui.status_menu.close")) { dismiss() }.buttonStyle(OutlineButtonStyle()).keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(width: 720, height: 560)
    }
    private func card(_ team: GalleryTeamView) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                Avatar(name: team.name, id: team.groupId, size: 36, shape: .square)
                VStack(alignment: .leading, spacing: 2) {
                    Text(team.name).font(.figtree(14, .semibold)).lineLimit(2)
                    Muted(team.private ? L("ui.teams_admin.private") : L("ui.teams_admin.public"), size: 11)
                }
            }
            if !team.description.isEmpty {
                Text(team.description).font(.figtree(12)).foregroundStyle(Theme.textSoft).lineLimit(3)
            }
            if team.knownMembers > 0 {
                Muted(L("ui.teams_admin.known_members", ["count": team.knownMembers]), size: 11)
            }
            Spacer(minLength: 0)
            switch team.join {
            case .none:
                Button(team.private ? L("ui.teams_admin.request_to_join") : L("ui.teams_admin.join")) {
                    store.send(.teamsAdmin(action: .joinTeam(groupId: team.groupId)))
                }
                .buttonStyle(OutlineButtonStyle())
            case .joining:
                ProgressView().controlSize(.small)
            case .joined:
                Label(L("ui.teams_admin.joined"), systemImage: "checkmark").font(.figtree(12)).foregroundStyle(Theme.accent)
            case .requested:
                Label(L("ui.teams_admin.request_sent"), systemImage: "clock").font(.figtree(12)).foregroundStyle(Theme.muted)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 150, alignment: .topLeading)
        .card(padding: 14)
    }
}

/// The team's join code, which owners can generate again.
struct JoinCodeSheet: View {
    @Environment(Store.self) private var store
    let view: JoinCodeView
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(L("ui.teams_admin.team_code_of", ["team": view.teamName])).font(.figtree(17, .semibold))
            if view.loading {
                ProgressView()
            } else if let code = view.code {
                HStack {
                    Text(code).font(.system(size: 22, weight: .semibold, design: .monospaced)).textSelection(.enabled)
                    Spacer()
                    Button(L("ui.teams_admin.copy")) { NSPasteboard.copy(code) }
                        .buttonStyle(OutlineButtonStyle())
                }
                Muted(L("ui.teams_admin.code_explanation"), size: 12)
            } else {
                Muted(L("ui.teams_admin.no_code"), size: 12)
            }
            AdminError()
            HStack {
                Button(view.code == nil ? L("ui.teams_admin.generate") : L("ui.teams_admin.reset_code")) {
                    store.send(.teamsAdmin(action: .resetJoinCode))
                }
                .buttonStyle(OutlineButtonStyle()).disabled(view.loading)
                Spacer()
                Button(L("ui.status_menu.close")) { store.send(.teamsAdmin(action: .closeJoinCode)) }
                    .buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 400)
    }
}
