import SwiftUI
import OxideCore

/// The compose box's apps, as the web client's message extension flyout: pick an app, search or fill its form, insert a card.
struct ExtensionsFlyout: View {
    @Environment(Store.self) private var store
    let close: () -> Void
    @State private var opened: UInt32?

    private var view: ExtensionsView? { store.extensions }
    private var app: ExtensionAppView? {
        guard let id = view?.appId else { return nil }
        return view?.apps.first { $0.id == id }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let app { chosen(app) } else { apps }
        }
        .padding(12)
        .frame(width: 400, height: 440, alignment: .top)
        .onAppear {
            opened = view?.inserted ?? 0
            send(.load)
        }
        .onChange(of: view?.inserted) { _, inserted in
            if let inserted, let opened, inserted > opened { close() }
        }
    }

    private func send(_ action: ExtensionsAction) { store.send(.extensions(action: action)) }

    private var apps: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(L("ui.extensions.apps")).font(.figtree(14, .semibold))
            if view?.appsLoading ?? true {
                HStack { ProgressView().controlSize(.small); Muted(L("ui.extensions.loading_apps"), size: 12) }
            } else if let error = view?.appsError {
                Muted(error, size: 12).fixedSize(horizontal: false, vertical: true)
            } else if view?.apps.isEmpty ?? true {
                Muted(L("ui.extensions.no_apps"), size: 12).fixedSize(horizontal: false, vertical: true)
                if let note = view?.appsNote {
                    Text(note).font(.figtree(11)).foregroundStyle(Theme.subtle)
                        .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(view?.apps ?? [], id: \.id) { app in
                        Button {
                            if let first = app.commands.first { send(.choose(appId: app.id, commandId: first.id)) }
                        } label: {
                            HStack(spacing: 10) {
                                AppIcon(url: app.icon, side: 28)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(app.name).font(.figtree(13, .semibold))
                                    Muted(app.commands.map(\.title).joined(separator: " · "), size: 11).lineLimit(1)
                                }
                                Spacer()
                            }
                            .padding(6)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain).pointerCursor()
                        .accessibilityLabel(app.name)
                    }
                }
            }
        }
    }

    @ViewBuilder private func chosen(_ app: ExtensionAppView) -> some View {
        HStack(spacing: 8) {
            IconButton(symbol: "chevron.left", help: L("ui.extensions.back"), side: 26, glyph: 12) { send(.back) }
            AppIcon(url: app.icon, side: 22)
            Text(app.name).font(.figtree(14, .semibold)).lineLimit(1)
            Spacer()
            if view?.inserting == true { ProgressView().controlSize(.small) }
        }
        if app.commands.count > 1 {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    ForEach(app.commands, id: \.id) { command in
                        let selected = view?.commandId == command.id
                        Button { send(.choose(appId: app.id, commandId: command.id)) } label: {
                            Label(command.title, systemImage: command.action ? "square.and.pencil" : "magnifyingglass")
                                .font(.figtree(12, selected ? .semibold : .regular))
                                .padding(.horizontal, 8).padding(.vertical, 4)
                                .background(selected ? Theme.accentSoft : .clear, in: RoundedRectangle(cornerRadius: 6))
                        }
                        .buttonStyle(.plain).pointerCursor()
                        .help(command.description.isEmpty ? command.title : command.description)
                    }
                }
            }
        }
        if let form = view?.form {
            ExtensionForm(form: form, send: send)
        } else if let command = app.commands.first(where: { $0.id == view?.commandId }), !command.action {
            FileSearchField(placeholder: command.description.isEmpty ? L("ui.extensions.search") : command.description,
                            current: view?.query ?? "", delay: .milliseconds(400)) { send(.search(query: $0)) }
                .id(command.id)
            results
        } else {
            status
            Spacer()
        }
    }

    @ViewBuilder private var status: some View {
        if view?.loading == true || view?.inserting == true {
            HStack { ProgressView().controlSize(.small) }.frame(maxWidth: .infinity).padding(.top, 8)
        }
        if let message = view?.message {
            Muted(message, size: 12).fixedSize(horizontal: false, vertical: true)
        }
        if let link = view?.link {
            Button(link.title) { store.send(.openUrl(url: link.url)) }.buttonStyle(OutlineButtonStyle())
        }
    }

    private var results: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                status
                if view?.grid == true {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 110), spacing: 6)], spacing: 6) {
                        ForEach(view?.results ?? [], id: \.index) { item in
                            Button { send(.pick(index: item.index)) } label: {
                                VStack(spacing: 4) {
                                    if let image = item.image { RemoteThumbnail(url: image).frame(height: 80) }
                                    Text(item.title).font(.figtree(11)).lineLimit(2)
                                }
                            }
                            .buttonStyle(.plain).pointerCursor().help(item.text)
                        }
                    }
                } else {
                    ForEach(view?.results ?? [], id: \.index) { item in
                        Button { send(.pick(index: item.index)) } label: {
                            HStack(alignment: .top, spacing: 10) {
                                if let image = item.image { RemoteThumbnail(url: image).frame(width: 44, height: 44) }
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(item.title).font(.figtree(13, .semibold)).lineLimit(2)
                                    if !item.text.isEmpty { Muted(item.text, size: 12).lineLimit(2) }
                                }
                                Spacer(minLength: 0)
                            }
                            .padding(6)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain).pointerCursor()
                        .disabled(view?.inserting == true)
                        .accessibilityLabel(item.title)
                    }
                }
            }
        }
    }
}

/// An action command's form: its Adaptive Card, whose buttons go to the app rather than to a message.
private struct ExtensionForm: View {
    @Environment(Store.self) private var store
    let form: ExtensionFormView
    let send: (ExtensionsAction) -> Void
    @State private var values: [String: String] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !form.title.isEmpty { Text(form.title).font(.figtree(13, .semibold)) }
            if let card = form.card {
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        CardItems(items: card.items, binding: binding)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                HStack(spacing: 8) {
                    ForEach(Array(card.actions.enumerated()), id: \.offset) { index, action in
                        let button = Button(action.title) { run(card, index: index) }
                        Group {
                            if index == 0 { button.buttonStyle(PrimaryButtonStyle()) } else { button.buttonStyle(OutlineButtonStyle()) }
                        }
                        .disabled(form.busy || action.kind == .unsupported)
                    }
                    Spacer()
                    Button(L("ui.extensions.cancel")) { send(.closeForm) }.buttonStyle(OutlineButtonStyle())
                    if form.busy { ProgressView().controlSize(.small) }
                }
            } else if let url = form.url {
                Muted(L("ui.extensions.web_form"), size: 12).fixedSize(horizontal: false, vertical: true)
                Button(L("ui.extensions.open_in_browser")) { store.send(.openUrl(url: url)) }
                    .buttonStyle(OutlineButtonStyle())
                Spacer()
            }
            if let error = form.error {
                Text(error).font(.figtree(12)).foregroundStyle(Theme.errorText).fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func binding(_ input: CardInputView) -> Binding<String> {
        Binding(get: { values[input.id] ?? input.value }, set: { values[input.id] = $0 })
    }
    private func run(_ card: CardView, index: Int) {
        let inputs = CardItems.inputs(card.items).map { CardInputValue(id: $0.id, value: values[$0.id] ?? $0.value) }
        send(.formAction(action: UInt32(index), inputs: inputs))
    }
}

/// An app's catalog icon, or a generic one.
private struct AppIcon: View {
    let url: String?
    let side: CGFloat
    var body: some View {
        Group {
            if let url { RemoteThumbnail(url: url) } else { Image(systemName: "square.grid.2x2").foregroundStyle(Theme.muted) }
        }
        .frame(width: side, height: side)
        .clipShape(RoundedRectangle(cornerRadius: 6))
    }
}

/// Cards from message extensions waiting in the draft, sent with it.
struct ComposerCards: View {
    @Environment(Store.self) private var store
    let cards: [ComposerCardView]
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(cards, id: \.index) { item in
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Muted(L("ui.extensions.via", ["app": item.appName]), size: 11)
                        Spacer()
                        IconButton(symbol: "xmark", help: L("ui.extensions.remove_card"), side: 22, glyph: 10) {
                            store.send(.extensions(action: .removeCard(index: item.index)))
                        }
                    }
                    MessageBody(blocks: [.card(card: item.card)], size: 13)
                }
            }
        }
    }
}
