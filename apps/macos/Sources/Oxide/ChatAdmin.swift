import SwiftUI
import OxideCore

/// Name, "Add people" and "Leave" of the open group chat, above its participants.
struct GroupChatControls: View {
    @Environment(Store.self) private var store
    let view: MembersView
    @State private var name = ""
    @State private var confirmLeave = false
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(L("ui.chat_admin.group_name")).font(.figtree(13, .semibold))
            HStack {
                TextField(L("ui.chat_admin.name_the_chat"), text: $name)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(rename)
                Button(L("ui.chat_admin.save"), action: rename)
                    .buttonStyle(OutlineButtonStyle())
                    .disabled(name.trimmingCharacters(in: .whitespaces) == view.chatName || store.chatAdmin?.busy == true)
            }
            HStack(spacing: 10) {
                Button {
                    store.send(.chatAdmin(action: .startAdding))
                } label: {
                    Label(L("ui.chat_admin.add_people"), systemImage: "person.badge.plus")
                }
                .buttonStyle(PrimaryButtonStyle())
                Button(L("ui.chat_admin.leave"), role: .destructive) { confirmLeave = true }
                    .buttonStyle(OutlineButtonStyle())
                if store.chatAdmin?.busy == true { ProgressView().controlSize(.small) }
            }
        }
        .card(padding: 16)
        .onAppear { name = view.chatName }
        .onChange(of: view.chatName) { _, value in name = value }
        .confirmationDialog(L("ui.chat_admin.leave_question", ["title": view.chatName.isEmpty ? L("ui.chat_admin.this_chat") : view.chatName]),
                            isPresented: $confirmLeave) {
            Button(L("ui.chat_admin.leave"), role: .destructive) {
                if let id = store.shell?.selected?.id { store.send(.chatAdmin(action: .leave(chatId: id))) }
            }
        } message: {
            Text(L("ui.chat_admin.leave_explanation"))
        }
    }
    private func rename() {
        store.send(.chatAdmin(action: .rename(title: name)))
    }
}

/// "Add people": search the directory, pick people, choose how much history they see.
struct AddPeopleSheet: View {
    @Environment(Store.self) private var store
    let adding: AddPeopleView
    @State private var query = ""
    @State private var days = 1
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(L("ui.chat_admin.add_people")).font(.figtree(17, .semibold))
            if !adding.chosen.isEmpty {
                FlowChips(people: adding.chosen) { store.send(.chatAdmin(action: .unchoose(mri: $0.mri))) }
            }
            HStack {
                TextField(L("ui.chat_admin.enter_a_name_or_email"), text: $query)
                    .textFieldStyle(.roundedBorder)
                    .onChange(of: query) { _, value in store.send(.chatAdmin(action: .addQuery(query: value))) }
                    .onSubmit { store.send(.chatAdmin(action: .addSearch)) }
                Button(adding.searching ? L("ui.chat_admin.searching") : L("ui.chat_admin.search")) {
                    store.send(.chatAdmin(action: .addSearch))
                }
                .buttonStyle(OutlineButtonStyle())
                .disabled(adding.searching || query.trimmingCharacters(in: .whitespaces).count < 2)
            }
            if !adding.results.isEmpty {
                ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(adding.results, id: \.mri) { person in
                            Button { store.send(.chatAdmin(action: .choose(mri: person.mri))) } label: {
                                HStack(spacing: 10) {
                                    Avatar(name: person.name, id: person.mri, size: 28)
                                    VStack(alignment: .leading, spacing: 1) {
                                        Text(person.name).font(.figtree(13))
                                        if !person.email.isEmpty { Muted(person.email, size: 11) }
                                    }
                                    Spacer()
                                }
                                .padding(6).contentShape(Rectangle())
                            }
                            .buttonStyle(GhostButtonStyle())
                        }
                    }
                }
                .frame(maxHeight: 200)
            } else if adding.searched {
                Muted(L("ui.chat_admin.nobody_found"), size: 12)
            }
            Divider()
            Picker(L("ui.chat_admin.history"), selection: Binding(
                get: { adding.history },
                set: { store.send(.chatAdmin(action: .setHistory(history: $0, days: UInt32(days)))) }
            )) {
                Text(L("ui.chat_admin.no_history")).tag(ShareHistory.none)
                Text(L("ui.chat_admin.history_days")).tag(ShareHistory.days)
                Text(L("ui.chat_admin.all_history")).tag(ShareHistory.all)
            }
            .pickerStyle(.radioGroup)
            if adding.history == .days {
                Stepper(value: Binding(get: { days }, set: {
                    days = $0
                    store.send(.chatAdmin(action: .setHistory(history: .days, days: UInt32($0))))
                }), in: 1...365) {
                    Text(L("ui.chat_admin.days_count", ["count": days])).font(.figtree(13))
                }
            }
            HStack {
                Spacer()
                Button(L("ui.chat_admin.cancel")) { store.send(.chatAdmin(action: .cancelAdding)) }
                    .buttonStyle(OutlineButtonStyle()).keyboardShortcut(.cancelAction)
                Button(adding.adding ? L("ui.chat_admin.adding") : L("ui.chat_admin.add")) {
                    store.send(.chatAdmin(action: .confirmAdd))
                }
                .buttonStyle(PrimaryButtonStyle())
                .disabled(adding.chosen.isEmpty || adding.adding)
            }
        }
        .padding(20)
        .frame(width: 460)
        .onAppear { query = adding.query; days = Int(max(adding.days, 1)) }
    }
}

/// Removable chips of chosen people.
struct FlowChips: View {
    let people: [PersonView]
    let remove: (PersonView) -> Void
    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(people, id: \.mri) { person in
                    HStack(spacing: 4) {
                        Text(person.name).font(.figtree(12))
                        Button { remove(person) } label: { Image(systemName: "xmark").font(.system(size: 9)) }
                            .buttonStyle(.plain)
                            .accessibilityLabel(L("ui.chat_admin.remove_person", ["name": person.name]))
                    }
                    .padding(.horizontal, 8).padding(.vertical, 4)
                    .background(Theme.accentSoft, in: Capsule())
                }
            }
        }
    }
}
