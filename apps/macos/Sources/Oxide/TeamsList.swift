import SwiftUI
import UniformTypeIdentifiers
import OxideCore

struct TeamsList: View {
    @Environment(Store.self) private var store
    let searching: Bool
    let loading: Bool
    let sidebar: Sidebar
    @State private var newChannelTeam: TeamRow?
    @State private var leaving: TeamRow?
    private var controls: TeamListControls { sidebar.teamControls }
    private func send(_ action: TeamListAction) { store.send(.teamList(action: action)) }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(L("ui.teams_list.shown_hidden", ["shownCount": controls.shownCount, "hiddenCount": controls.hiddenCount])).font(.figtree(11)).foregroundStyle(Theme.muted)
                Spacer()
                Menu {
                    Button { send(.sort(sort: .teams)) } label: { Label(L("ui.teams_list.teams_order"), systemImage: controls.sort == .teams ? "checkmark" : "list.bullet") }
                    Button { send(.sort(sort: .alphabetical)) } label: { Label(L("ui.teams_list.a_to_z"), systemImage: controls.sort == .alphabetical ? "checkmark" : "textformat.abc") }
                } label: { Image(systemName: "arrow.up.arrow.down").frame(width: 28, height: 26) }
                .menuStyle(.borderlessButton).fixedSize().pointerCursor().help(L("ui.teams_list.sort_teams")).accessibilityLabel(L("ui.teams_list.sort_teams"))
            }.padding(.horizontal, 16)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 3) {
                    ForEach(sidebar.teams.filter { !$0.hidden }, id: \.id) { team in teamSection(team) }
                    if !searching && controls.shownCount == 0 && controls.hiddenCount > 0 {
                        Muted(L("ui.teams_list.all_your_teams_are_hidden"), size: 12).padding(12)
                    }
                    if controls.hiddenCount > 0 && !searching {
                        Button { send(.hiddenTeams(expanded: !controls.showHidden)) } label: {
                            HStack {
                                Image(systemName: controls.showHidden ? "chevron.down" : "chevron.right").font(.system(size: 10, weight: .semibold))
                                Text(L("ui.teams_list.hidden_teams", ["hiddenCount": controls.hiddenCount])).font(.figtree(12, .semibold))
                                Spacer()
                            }.padding(10).contentShape(Rectangle())
                        }.buttonStyle(GhostButtonStyle()).padding(.top, 8)
                    }
                    ForEach(sidebar.teams.filter(\.hidden), id: \.id) { team in teamSection(team) }
                    if sidebar.teamCount == 0 && loading && !searching {
                        SkeletonList(rows: 6, avatar: 28)
                            .accessibilityElement().accessibilityLabel(L("ui.teams_list.loading_teams"))
                    } else if sidebar.teamCount == 0 || (searching && sidebar.teams.isEmpty) {
                        Muted(loading ? L("ui.teams_list.loading_teams")
                              : sidebar.teamsProblem != nil ? L("ui.teams_list.couldn_t_load_teams")
                              : searching ? L("ui.teams_list.no_matching_teams_or_channels")
                              : L("ui.teams_list.no_teams_available")).padding(16)
                    }
                    if let problem = sidebar.teamsProblem {
                        VStack(alignment: .leading, spacing: 6) {
                            Muted(L("ui.teams_list.the_team_list_couldn_t_be"), size: 11)
                            Button(L("ui.teams_list.try_again")) { store.send(.refresh) }.buttonStyle(OutlineButtonStyle())
                        }.padding(12).help(problem)
                    }
                    if controls.savingOrder { HStack { ProgressView().controlSize(.small); Muted(L("ui.teams_list.saving_order"), size: 11) }.padding(12) }
                }.padding(.horizontal, 8).padding(.vertical, 10)
            }
        }
        .sheet(item: $newChannelTeam) { team in NewChannelSheet(team: team) }
        .sheet(isPresented: Binding(
            get: { store.teamsAdmin?.joinCode != nil },
            set: { if !$0 { store.send(.teamsAdmin(action: .closeJoinCode)) } }
        )) {
            if let code = store.teamsAdmin?.joinCode { JoinCodeSheet(view: code) }
        }
        .confirmationDialog(L("ui.teams_admin.leave_question", ["name": leaving?.name ?? ""]),
                            isPresented: Binding(get: { leaving != nil }, set: { if !$0 { leaving = nil } })) {
            Button(L("ui.teams_admin.leave_team"), role: .destructive) {
                if let team = leaving { store.send(.teamsAdmin(action: .leaveTeam(teamId: team.id))) }
                leaving = nil
            }
        }
    }
    private func teamSection(_ team: TeamRow) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            teamRow(team)
            if team.showChannels {
                ForEach(team.channels.filter { !$0.hidden }, id: \.id) { TeamChannelRow(channel: $0) }
                if team.hiddenChannelCount > 0 && !searching {
                    more(team.showHiddenChannels
                         ? L("ui.teams_list.hide_hidden_channels", ["hiddenChannelCount": team.hiddenChannelCount])
                         : L("ui.teams_list.see_hidden_channels", ["hiddenChannelCount": team.hiddenChannelCount]), indent: 36, context: team.name) {
                        send(.hiddenChannels(team: team.id, expanded: !team.showHiddenChannels))
                    }
                }
                ForEach(team.channels.filter(\.hidden), id: \.id) { TeamChannelRow(channel: $0) }
                if team.loading { LoadingLabel(text: L("ui.teams_list.loading_channels"), size: 11).padding(.leading, 36).padding(.vertical, 6) }
                if team.failed {
                    VStack(alignment: .leading, spacing: 4) {
                        Muted(L("ui.teams_list.channels_couldn_t_be_refreshed"), size: 11)
                        Button(L("ui.teams_list.try_again")) { store.send(.loadChannels(id: team.id)) }.buttonStyle(GhostButtonStyle(accent: true))
                    }.padding(.leading, 36).padding(.vertical, 6)
                } else if team.channels.isEmpty && !team.loading && team.hiddenChannelCount == 0 {
                    Muted(searching ? L("ui.teams_list.no_matching_channels") : L("ui.teams_list.no_channels_available"), size: 11).padding(.leading, 36).padding(.vertical, 6)
                }
            }
        }
    }
    private func teamRow(_ team: TeamRow) -> some View {
        TeamHeaderRow(team: team) { teamMenu(team) }
        .contextMenu { teamMenu(team) }
        .modifier(TeamDrag(id: team.id, enabled: canMove))
        .onDrop(of: [UTType.plainText], isTargeted: nil) { providers in
            guard canMove, let provider = providers.first else { return false }
            let peers = sidebar.teams.filter { $0.hidden == team.hidden }.map(\.id)
            provider.loadObject(ofClass: NSString.self) { value, _ in
                guard let id = value as? String, id != team.id, peers.contains(id) else { return }
                DispatchQueue.main.async { send(.move(id: id, before: team.id)) }
            }
            return true
        }
    }
    @ViewBuilder private func teamMenu(_ team: TeamRow) -> some View {
        Button(team.hidden ? L("ui.teams_list.show_team") : L("ui.teams_list.hide_team")) { send(.show(team: team.id, channel: nil, shown: team.hidden)) }.disabled(team.pending)
        Button(team.showHiddenChannels ? L("ui.teams_list.hide_the_list_of_hidden_channels") : L("ui.teams_list.see_all_hidden_channels")) {
            if !team.showHiddenChannels && !team.expanded { store.send(.toggleTeam(id: team.id)) }
            send(.hiddenChannels(team: team.id, expanded: !team.showHiddenChannels))
        }
        Divider()
        Button(L("ui.teams_list.move_up")) { send(.shift(id: team.id, up: true)) }.disabled(!canShift(team, up: true))
        Button(L("ui.teams_list.move_down")) { send(.shift(id: team.id, up: false)) }.disabled(!canShift(team, up: false))
        Divider()
        Button(L("ui.teams_list.refresh_channels")) { store.send(.loadChannels(id: team.id)) }
        if !team.archived {
            Divider()
            Button(L("ui.teams_admin.add_channel")) { newChannelTeam = team }
            Button(L("ui.teams_admin.team_code")) { store.send(.teamsAdmin(action: .showJoinCode(teamId: team.id))) }
            Button(L("ui.teams_admin.leave_team"), role: .destructive) { leaving = team }
        }
    }
    private func canShift(_ team: TeamRow, up: Bool) -> Bool {
        guard canMove else { return false }
        let peers = sidebar.teams.filter { $0.hidden == team.hidden }
        return up ? peers.first?.id != team.id : peers.last?.id != team.id
    }
    private var canMove: Bool { controls.orderAvailable && !controls.savingOrder && controls.sort == .teams && !searching }
    private func more(_ title: String, indent: CGFloat, context: String? = nil, action: @escaping () -> Void) -> some View {
        Button(title, action: action).accessibilityLabel(context.map { "\(title) · \($0)" } ?? title).font(.figtree(11)).buttonStyle(GhostButtonStyle(accent: true)).padding(.leading, indent).padding(.vertical, 5)
    }
}

private struct TeamDrag: ViewModifier {
    let id: String
    let enabled: Bool
    @ViewBuilder func body(content: Content) -> some View {
        if enabled { content.onDrag { NSItemProvider(object: id as NSString) } } else { content }
    }
}

private struct TeamHeaderRow<Options: View>: View {
    @Environment(Store.self) private var store
    let team: TeamRow
    @ViewBuilder let options: () -> Options
    @State private var hovered = false
    @FocusState private var focused: Bool
    private var showOptions: Bool { hovered || focused }
    private var description: String {
        var values = [team.name]
        if team.hidden { values.append(L("ui.teams_list.hidden")) }
        if team.unread { values.append(L("ui.teams_list.unread")) }
        if team.archived { values.append(L("ui.teams_list.archived")) }
        return values.joined(separator: " · ")
    }
    var body: some View {
        HStack(spacing: 0) {
            Button { store.send(.toggleTeam(id: team.id)) } label: {
                HStack(spacing: 8) {
                    Image(systemName: team.showChannels ? "chevron.down" : "chevron.right").font(.system(size: 10, weight: .semibold)).foregroundStyle(Theme.subtle).frame(width: 12)
                    Avatar(name: team.name, id: team.id, size: 30, shape: .square)
                    Text(team.name).font(.figtree(13, team.unread ? .bold : .semibold)).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                    // Collapsed: the channels' dots are out of sight.
                    if team.unread && !team.showChannels {
                        Circle().fill(Theme.accent).frame(width: 8, height: 8).accessibilityHidden(true)
                    }
                }.padding(.vertical, 8).padding(.leading, 6).contentShape(Rectangle())
            }
            .buttonStyle(.plain).pointerCursor().focused($focused)
            .accessibilityLabel(description).accessibilityValue(team.showChannels ? L("ui.teams_list.expanded") : L("ui.teams_list.collapsed"))
            // The menu is only built while it can be used; a placeholder keeps the row's layout.
            if showOptions {
                Menu { options() } label: { Image(systemName: "ellipsis").frame(width: 26, height: 28) }
                    .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().pointerCursor().disabled(team.pending)
                    .accessibilityLabel(L("ui.teams_list.options_for_team", ["name": team.name]))
            } else {
                Color.clear.frame(width: 26, height: 28).accessibilityHidden(true)
            }
        }
        .background(hovered ? Theme.input : .clear, in: RoundedRectangle(cornerRadius: 7))
        .contentShape(Rectangle())
        .onHover { if hovered != $0 { hovered = $0 } }
        .help(description)
    }
}

/// Keep hover state local so moving between channels does not redraw the whole list.
private struct TeamChannelRow: View {
    @Environment(Store.self) private var store
    let channel: ChannelRow
    @State private var hovered = false
    @FocusState private var focused: Bool
    private var showOptions: Bool { hovered || focused }
    private var description: String {
        var values = [channel.name]
        if channel.kind.lowercased() == "private" { values.append(L("ui.teams_list.private_channel")) }
        if channel.kind.lowercased() == "shared" { values.append(L("ui.teams_list.shared_channel")) }
        if channel.hidden { values.append(L("ui.teams_list.hidden_2")) }
        if channel.unread { values.append(L("ui.teams_list.unread")) }
        if channel.archived { values.append(L("ui.teams_list.archived_2")) }
        return values.joined(separator: " · ")
    }
    var body: some View {
        HStack(spacing: 0) {
            Button { store.send(.select(target: Target(team: channel.team, id: channel.id))) } label: {
                HStack(spacing: 7) {
                    Image(systemName: channel.kind.lowercased() == "private" ? "lock" : channel.kind.lowercased() == "shared" ? "arrow.triangle.branch" : "number")
                        .font(.system(size: 12)).foregroundStyle(channel.selected ? Theme.accent : Theme.subtle).frame(width: 14)
                    Text(channel.name).font(.figtree(13, channel.unread ? .semibold : .regular)).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                    if channel.muted {
                        Image(systemName: "bell.slash").font(.system(size: 10)).foregroundStyle(Theme.subtle)
                            .accessibilityLabel(L("ui.teams_list.notifications_turned_off"))
                    }
                    if channel.unread {
                        Circle().fill(Theme.accent).frame(width: 8, height: 8).accessibilityHidden(true)
                    }
                }.foregroundStyle(channel.hidden ? Theme.muted : Theme.text).padding(.leading, 8).padding(.vertical, 7).contentShape(Rectangle())
            }
            .buttonStyle(.plain).pointerCursor().focused($focused)
            .accessibilityLabel(description).accessibilityAddTraits(channel.selected ? .isSelected : [])
            if showOptions {
                Menu { options } label: { Image(systemName: "ellipsis").frame(width: 24, height: 26) }
                    .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().pointerCursor()
                    .disabled(channel.pending).accessibilityLabel(L("ui.teams_list.options_for_channel", ["name": channel.name]))
            } else {
                Color.clear.frame(width: 24, height: 26).accessibilityHidden(true)
            }
        }
        .background(channel.selected ? Theme.accentSoft : hovered ? Theme.input : .clear, in: RoundedRectangle(cornerRadius: 7))
        .contentShape(Rectangle())
        .onHover { if hovered != $0 { hovered = $0 } }
        .contextMenu { options }.help(description).padding(.leading, 28)
    }
    @ViewBuilder private var options: some View {
        if channel.archived { Text(L("ui.teams_list.archived_channel")) }
        Button(channel.hidden ? L("ui.teams_list.show_channel") : L("ui.teams_list.hide_channel")) {
            store.send(.teamList(action: .show(team: channel.team, channel: channel.id, shown: channel.hidden)))
        }.disabled(channel.pending)
        Divider()
        ChannelNotificationMenu(team: channel.team, channel: channel.id, muted: channel.muted)
    }
}
