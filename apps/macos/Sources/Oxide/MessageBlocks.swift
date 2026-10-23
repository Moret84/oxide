import SwiftUI
import OxideCore

/// A message body drawn block by block, as Teams does: each paragraph, quote, code, list, table or card has its own view.
struct MessageBlocks: View {
    let blocks: [BlockView]
    var size: Double = 14

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                BlockContent(block: block, size: size)
            }
        }
        .frame(alignment: .leading)
    }

    /// Images drawn in the body, where the HTML puts them.
    static func images(_ blocks: [BlockView]) -> [ImageView] {
        blocks.flatMap { block -> [ImageView] in
            switch block {
            case .image(let image): [image]
            case .list(_, _, let items): items.flatMap { images($0.blocks) }
            case .quote(_, _, _, let body): images(body)
            default: []
            }
        }
    }

    /// One to three emoji alone are drawn large, like Teams.
    static func emojiOnly(_ blocks: [BlockView]) -> Bool {
        guard blocks.count == 1, case .text(let runs) = blocks[0] else { return false }
        let glyphs = runs.map(\.text).joined().filter { !$0.isWhitespace }
        return (1...3).contains(glyphs.count) && glyphs.allSatisfy(\.isEmojiGlyph)
    }
}

private struct BlockContent: View {
    let block: BlockView
    let size: Double

    var body: some View {
        switch block {
        case .text(let runs):
            MessageEmojiText(runs: runs, size: size)
                .lineSpacing(size > 20 ? 0 : 4)
                .fixedSize(horizontal: false, vertical: true)
        case .list(let ordered, let start, let items):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(ordered ? "\(Int(start) + index)." : "•")
                            .font(.figtree(size))
                            .foregroundStyle(Theme.textSoft)
                            .frame(minWidth: 14, alignment: .trailing)
                        MessageBlocks(blocks: item.blocks, size: size)
                    }
                }
            }
            .padding(.leading, 2)
        case .quote(let author, let time, let messageId, let body):
            QuoteBlock(author: author, time: time, reply: messageId != nil || author != nil, content: body)
        case .code(let language, let text):
            CodeBlock(language: language, text: text)
        case .table(let header, let rows):
            TableBlock(header: header, rows: rows, size: size - 1)
        case .rule:
            Rectangle().fill(Theme.border).frame(height: 1).padding(.vertical, 2)
        case .image(let image):
            PreviewImage(image: image)
        case .card(let card):
            CardBlock(card: card)
        }
    }
}

/// A reply to another message: its author, time and the start of its text, set off by a bar.
private struct QuoteBlock: View {
    let author: String?
    let time: String?
    let reply: Bool
    let content: [BlockView]

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            if author != nil || time != nil {
                HStack(spacing: 6) {
                    if let author { Text(author).font(.figtree(12, .semibold)) }
                    if let time { Text(time).font(.figtree(11)).foregroundStyle(Theme.muted) }
                }
            }
            MessageBlocks(blocks: content, size: 13)
                .foregroundStyle(Theme.textSoft)
                .lineLimit(reply ? 4 : nil)
        }
        .padding(.vertical, 6)
        .padding(.leading, 12)
        .padding(.trailing, 10)
        .frame(alignment: .leading)
        .background(Theme.input, in: RoundedRectangle(cornerRadius: 6))
        .overlay(alignment: .leading) {
            UnevenRoundedRectangle(topLeadingRadius: 6, bottomLeadingRadius: 6)
                .fill(Theme.accent)
                .frame(width: 3)
        }
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(reply ? (author.map { L("ui.message_blocks.reply_to", ["author": $0]) } ?? L("ui.message_blocks.reply_to_message")) : L("ui.message_blocks.quote"))
    }
}

/// Code as written: monospaced, never wrapped (scrolls sideways), with its language and a copy button.
private struct CodeBlock: View {
    let language: String?
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(language ?? "Code").font(.figtree(11, .semibold)).foregroundStyle(Theme.muted)
                Spacer()
                Button {
                    NSPasteboard.copy(text)
                } label: {
                    Label(L("ui.message_blocks.copy"), systemImage: "doc.on.doc").font(.figtree(11))
                }
                .buttonStyle(GhostButtonStyle())
                .help(L("ui.message_blocks.copy_code"))
            }
            .padding(.leading, 10)
            .padding(.trailing, 4)
            .frame(height: 26)
            Rectangle().fill(Theme.border).frame(height: 1)
            ScrollView(.horizontal) {
                Text(text)
                    .font(.system(size: 12.5, design: .monospaced))
                    .lineSpacing(2)
                    .textSelection(.enabled)
                    .fixedSize()
                    .padding(10)
            }
            .fixedSize(horizontal: false, vertical: true)
        }
        .background(Theme.input, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.border))
    }
}

private struct TableBlock: View {
    let header: Bool
    let rows: [TableRowView]
    let size: Double

    var body: some View {
        Grid(alignment: .topLeading, horizontalSpacing: 0, verticalSpacing: 0) {
            ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                if index > 0 {
                    Rectangle().fill(Theme.border).frame(height: 1).gridCellUnsizedAxes(.horizontal)
                }
                GridRow {
                    ForEach(Array(row.cells.enumerated()), id: \.offset) { _, cell in
                        MessageEmojiText(runs: cell.runs, size: size)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .frame(minWidth: 48, maxWidth: .infinity, alignment: .leading)
                    }
                }
                .background(header && index == 0 ? Theme.input : .clear)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.border))
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// The message a card belongs to, for its buttons.
private struct CardMessageKey: EnvironmentKey { static let defaultValue: String? = nil }
extension EnvironmentValues {
    var cardMessageId: String? {
        get { self[CardMessageKey.self] }
        set { self[CardMessageKey.self] = newValue }
    }
}

/// A bot or connector card; typed input values stay here until a button sends them to the bot.
private struct CardBlock: View {
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
                MessageBlocks(blocks: blocks, size: 14)
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

private extension Character {
    var isEmojiGlyph: Bool {
        guard let first = unicodeScalars.first else { return false }
        return first.properties.isEmojiPresentation || (first.properties.isEmoji && unicodeScalars.count > 1)
    }
}
