import SwiftUI
import OxideCore

/// The message a card belongs to, for its buttons.
private struct CardMessageKey: EnvironmentKey { static let defaultValue: String? = nil }
extension EnvironmentValues {
    var cardMessageId: String? {
        get { self[CardMessageKey.self] }
        set { self[CardMessageKey.self] = newValue }
    }
}

/// A bot or connector card; typed input values stay here until a button sends them to the bot.
struct CardBlock: View {
    @Environment(Store.self) private var store
    @Environment(\.cardMessageId) private var messageId
    let card: CardView
    @State private var values: [String: String] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let title = card.title {
                Text(title).font(.figtree(15, .semibold)).fixedSize(horizontal: false, vertical: true)
            }
            if let subtitle = card.subtitle {
                Muted(subtitle, size: 13).fixedSize(horizontal: false, vertical: true)
            }
            CardItems(items: card.items, binding: binding)
            if !card.actions.isEmpty {
                let layout = card.actions.count <= 3
                    ? AnyLayout(HStackLayout(spacing: 8)) : AnyLayout(VStackLayout(alignment: .leading, spacing: 6))
                layout {
                    ForEach(Array(card.actions.enumerated()), id: \.offset) { index, action in
                        Button(action.title) { run(action, index: index) }
                            .buttonStyle(OutlineButtonStyle())
                            .disabled(action.kind == .unsupported || card.busy || (action.kind == .submit && messageId == nil))
                            .help(action.url ?? (action.kind == .unsupported ? L("ui.message_blocks.only_in_teams") : action.title))
                    }
                    if card.busy { ProgressView().controlSize(.small) }
                }
                .padding(.top, 2)
            }
            if let footer = card.footer {
                Muted(footer, size: 12).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(12)
        .padding(.top, card.accent == nil ? 0 : 3)
        .frame(maxWidth: 440, alignment: .leading)
        .card(radius: 10)
        // A connector card's `themeColor`, as Teams draws it: a bar along the top edge.
        .overlay(alignment: .top) {
            if let accent = card.accent { Theme.rgb(accent).frame(height: 3) }
        }
        .clipShape(RoundedRectangle(cornerRadius: 10))
    }

    private func binding(_ input: CardInputView) -> Binding<String> {
        Binding(get: { values[input.id] ?? input.value }, set: { values[input.id] = $0 })
    }
    private func run(_ action: CardActionView, index: Int) {
        if let url = action.url {
            store.send(.openUrl(url: url))
            return
        }
        guard let messageId else { return }
        let inputs = CardItems.inputs(card.items).map { input in
            CardInputValue(id: input.id, value: values[input.id] ?? input.value)
        }
        store.send(.cardAction(messageId: messageId, card: card.index, action: UInt32(index), inputs: inputs))
    }
}

/// A card's items, top to bottom; a ColumnSet's columns side by side.
struct CardItems: View {
    let items: [CardItemView]
    let binding: (CardInputView) -> Binding<String>

    var body: some View {
        ForEach(Array(items.enumerated()), id: \.offset) { _, item in
            switch item {
            case .text(let runs, let muted):
                MessageEmojiText(runs: runs)
                    .foregroundStyle(muted ? Theme.muted : Theme.text)
                    .lineSpacing(3)
                    .fixedSize(horizontal: false, vertical: true)
            case .blocks(let blocks):
                MessageBody(blocks: blocks, size: 14, inFeedSelection: false)
            case .gap(let points, let separator):
                if separator {
                    Divider().overlay(Theme.border).padding(.vertical, CGFloat(points) / 2)
                } else {
                    Color.clear.frame(height: CGFloat(points))
                }
            case .facts(let facts):
                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 4) {
                    ForEach(Array(facts.enumerated()), id: \.offset) { _, fact in
                        GridRow {
                            Text(fact.title).font(.figtree(13, .semibold))
                            Text(fact.value).font(.figtree(13)).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
            case .image(let image):
                PreviewImage(image: image, height: 180)
            case .input(let input):
                CardInputField(input: input, value: binding(input))
            case .columns(let columns):
                HStack(alignment: .top, spacing: 8) {
                    ForEach(Array(columns.enumerated()), id: \.offset) { _, column in
                        sized(column.width, AnyView(
                            VStack(alignment: .leading, spacing: 8) { CardItems(items: column.items, binding: binding) }
                        ))
                    }
                }
            case .emphasis(let items):
                VStack(alignment: .leading, spacing: 8) { CardItems(items: items, binding: binding) }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Theme.input, in: RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.border))
            }
        }
    }
    /// `auto` takes what its content needs first; the others share the rest (weights alike).
    @ViewBuilder private func sized(_ width: CardColumnWidth, _ column: AnyView) -> some View {
        switch width {
        case .auto: column.layoutPriority(1)
        case .points(let points): column.frame(width: CGFloat(points), alignment: .leading)
        case .stretch, .weight: column.frame(maxWidth: .infinity, alignment: .leading)
        }
    }
    static func inputs(_ items: [CardItemView]) -> [CardInputView] {
        items.flatMap { item -> [CardInputView] in
            switch item {
            case .input(let input): [input]
            case .columns(let columns): columns.flatMap { inputs($0.items) }
            case .emphasis(let items): inputs(items)
            default: []
            }
        }
    }
}

/// One `Input.*` of a card, editing its value as the card's JSON expects it.
private struct CardInputField: View {
    let input: CardInputView
    @Binding var value: String
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let label = input.label {
                Text(label + (input.required ? " *" : "")).font(.figtree(12, .semibold))
            }
            field
        }
    }
    @ViewBuilder private var field: some View {
        switch input.kind {
        case .text(let placeholder, let multiline):
            TextField(placeholder, text: $value, axis: multiline ? .vertical : .horizontal)
                .textFieldStyle(.roundedBorder).lineLimit(multiline ? 3...6 : 1...1)
        case .number(let placeholder):
            TextField(placeholder, text: $value).textFieldStyle(.roundedBorder)
                .onChange(of: value) { _, new in
                    let kept = new.filter { $0.isNumber || $0 == "." || $0 == "-" }
                    if kept != new { value = kept }
                }
        case .date:
            DatePicker("", selection: dateBinding(format: "yyyy-MM-dd"), displayedComponents: .date).labelsHidden()
        case .time:
            DatePicker("", selection: dateBinding(format: "HH:mm"), displayedComponents: .hourAndMinute).labelsHidden()
        case .toggle(let title, let on, let off):
            Toggle(title, isOn: Binding(get: { value == on }, set: { value = $0 ? on : off }))
                .onAppear { if value.isEmpty { value = off } }
        case .choice(let choices, let multi):
            if multi {
                ForEach(choices, id: \.value) { choice in
                    Toggle(choice.title, isOn: Binding(
                        get: { selected.contains(choice.value) },
                        set: { on in
                            var set = selected
                            if on { set.append(choice.value) } else { set.removeAll { $0 == choice.value } }
                            value = choices.map(\.value).filter(set.contains).joined(separator: ",")
                        }
                    ))
                }
            } else {
                Picker("", selection: $value) {
                    Text(L("ui.message_blocks.select")).tag("")
                    ForEach(choices, id: \.value) { Text($0.title).tag($0.value) }
                }
                .labelsHidden()
            }
        }
    }
    private var selected: [String] { value.split(separator: ",").map(String.init) }
    private func dateBinding(format: String) -> Binding<Date> {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = format
        return Binding(
            get: { formatter.date(from: value) ?? Date() },
            set: { value = formatter.string(from: $0) }
        )
    }
}

enum MessageText {
    /// Styled runs as one attributed string; `size` is the base for runs without their own.
    static func attributed(_ runs: [RunView], size base: Double = 14) -> AttributedString {
        var result = AttributedString()
        for run in runs {
            var piece = AttributedString(run.text)
            let size = run.size == 0 ? (run.code ? base - 1 : base) : Double(run.size)
            var font = run.code ? Font.system(size: size, design: .monospaced) : Font.figtree(size)
            if run.bold || run.mention { font = font.weight(.semibold) }
            if run.italic { font = font.italic() }
            piece.font = font
            if run.underline { piece.underlineStyle = .single }
            if run.strike { piece.strikethroughStyle = .single }
            if let color = ComposerText.color(run.color) { piece.foregroundColor = Color(nsColor: color) }
            if let color = ComposerText.color(run.highlight) {
                piece.backgroundColor = Color(nsColor: color)
            } else if run.code {
                piece.backgroundColor = Theme.input
            }
            if run.mention { piece.foregroundColor = Theme.accentText }
            if let link = run.link, let url = URL(string: link) {
                piece.link = url
                piece.foregroundColor = Theme.accent
                piece.underlineStyle = .single
            }
            result += piece
        }
        return result
    }
}

extension Character {
    var isEmojiGlyph: Bool {
        guard let first = unicodeScalars.first else { return false }
        return first.properties.isEmojiPresentation || (first.properties.isEmoji && unicodeScalars.count > 1)
    }
}
