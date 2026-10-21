import SwiftUI
import OxideCore

struct FormattingToolbar: View {
    let input: DraftInput
    private var colors: [(String, String)] {
        [(L("ui.formatting_toolbar.black"), "#202020"), (L("ui.formatting_toolbar.gray"), "#666666"), (L("ui.formatting_toolbar.red"), "#c4314b"), ("Orange", "#c75200"),
         (L("ui.formatting_toolbar.yellow"), "#a66900"), (L("ui.formatting_toolbar.green"), "#237b4b"), (L("ui.formatting_toolbar.blue"), "#0078d4"), (L("ui.formatting_toolbar.purple"), "#744da9")]
    }
    private var highlights: [(String, String)] {
        [(L("ui.formatting_toolbar.yellow"), "#fff100"), (L("ui.formatting_toolbar.green"), "#92d050"), ("Cyan", "#00ffff"), (L("ui.formatting_toolbar.pink"), "#ff99cc"), (L("ui.formatting_toolbar.lavender"), "#d9c2f0")]
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 2) {
                tool("bold", L("ui.formatting_toolbar.bold_b"), active: input.currentFormat.bold) { input.toggle(.bold) }
                tool("italic", L("ui.formatting_toolbar.italic_i"), active: input.currentFormat.italic) { input.toggle(.italic) }
                tool("underline", L("ui.formatting_toolbar.underline_u"), active: input.currentFormat.underline) { input.toggle(.underline) }
                tool("strikethrough", L("ui.formatting_toolbar.strikethrough_x"), active: input.currentFormat.strike) { input.toggle(.strike) }
                Divider().frame(height: 18).padding(.horizontal, 3)
                Menu {
                    Button(L("ui.formatting_toolbar.automatic")) { input.change { $0.color = nil } }
                    ForEach(colors, id: \.1) { name, hex in
                        Button { input.change { $0.color = hex } } label: { Label(name, systemImage: "circle.fill").foregroundStyle(Color(nsColor: ComposerText.color(hex)!)) }
                    }
                } label: { Image(systemName: "a.underlined").frame(width: 26, height: 28) }
                .toolMenu(L("ui.formatting_toolbar.font_color"))
                Menu {
                    Button(L("ui.formatting_toolbar.no_highlight")) { input.change { $0.highlight = nil } }
                    ForEach(highlights, id: \.1) { name, hex in
                        Button(name) { input.change { $0.highlight = hex } }
                    }
                } label: { Image(systemName: "highlighter").frame(width: 26, height: 28) }
                .toolMenu(L("ui.formatting_toolbar.highlight_color"))
                Menu {
                    ForEach([10, 12, 14, 18, 24, 32], id: \.self) { size in
                        Button("\(size) pt") { input.change { $0.size = UInt32(size) } }
                    }
                } label: { Text("\(input.currentFormat.size == 0 ? 14 : input.currentFormat.size) pt").font(.figtree(12)).frame(minWidth: 44) }
                .toolMenu(L("ui.formatting_toolbar.font_size"))
                tool("textformat.alt", L("ui.formatting_toolbar.clear_all_formatting")) { input.clearFormatting() }
                Spacer(minLength: 0)
            }
            HStack(spacing: 2) {
                Menu {
                    Button(L("ui.formatting_toolbar.paragraph")) { input.setBlock(.paragraph) }
                    Button(L("ui.formatting_toolbar.heading_1")) { input.setBlock(.heading1) }
                    Button(L("ui.formatting_toolbar.heading_2")) { input.setBlock(.heading2) }
                    Button(L("ui.formatting_toolbar.heading_3")) { input.setBlock(.heading3) }
                } label: { Text(styleLabel).font(.figtree(12)).frame(minWidth: 70) }
                .toolMenu(L("ui.formatting_toolbar.paragraph_style"))
                tool("list.bullet", L("ui.formatting_toolbar.bulleted_list"), active: input.currentFormat.block == .bullet) { input.setBlock(.bullet) }
                tool("list.number", L("ui.formatting_toolbar.numbered_list"), active: input.currentFormat.block == .numbered) { input.setBlock(.numbered) }
                tool("decrease.indent", L("ui.formatting_toolbar.decrease_indent")) { input.indent(-1) }
                tool("increase.indent", L("ui.formatting_toolbar.increase_indent")) { input.indent(1) }
                tool("text.quote", L("ui.formatting_toolbar.quote"), active: input.currentFormat.block == .quote) { input.setBlock(.quote) }
                tool("link", L("ui.formatting_toolbar.insert_link_k"), active: input.currentFormat.link != nil) { input.linkRequested = true }
                    .popover(isPresented: Binding(get: { input.linkRequested }, set: { input.linkRequested = $0 })) {
                        ComposerLinkEditor(input: input)
                    }
                Menu {
                    Button(L("ui.formatting_toolbar.inline_code")) { input.toggle(.code) }
                    Button(L("ui.formatting_toolbar.code_block")) { input.setBlock(.codeBlock) }
                    Divider()
                    Button(L("ui.formatting_toolbar.align_left")) { input.setAlignment("left") }
                    Button(L("ui.formatting_toolbar.center")) { input.setAlignment("center") }
                    Button(L("ui.formatting_toolbar.align_right")) { input.setAlignment("right") }
                    Divider()
                    Button(L("ui.formatting_toolbar.horizontal_line")) { input.insertRule() }
                    Button(L("ui.formatting_toolbar.remove_link")) { input.change { $0.link = nil } }
                    Divider()
                    Button(L("ui.formatting_toolbar.undo")) { input.textView?.undoManager?.undo() }
                    Button(L("ui.formatting_toolbar.redo")) { input.textView?.undoManager?.redo() }
                } label: { Image(systemName: "ellipsis").frame(width: 26, height: 28) }
                .toolMenu(L("ui.formatting_toolbar.more_formatting_options"))
                Spacer(minLength: 0)
            }
        }
        .padding(.bottom, 4)
        .overlay(alignment: .bottom) { Divider().overlay(Theme.border) }
    }
    private var styleLabel: String {
        switch input.currentFormat.block {
        case .heading1: L("ui.formatting_toolbar.heading_1")
        case .heading2: L("ui.formatting_toolbar.heading_2")
        case .heading3: L("ui.formatting_toolbar.heading_3")
        default: L("ui.formatting_toolbar.paragraph")
        }
    }
    private func tool(_ symbol: String, _ label: String, active: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) { Image(systemName: symbol).font(.system(size: 13)).frame(width: 27, height: 28) }
            .buttonStyle(GhostButtonStyle(accent: active))
            .background(active ? Theme.accentSoft : .clear, in: RoundedRectangle(cornerRadius: 5))
            .help(label).accessibilityLabel(label)
    }
}

private extension View {
    func toolMenu(_ label: String) -> some View {
        menuStyle(.borderlessButton).fixedSize().help(label).accessibilityLabel(label).pointerCursor()
    }
}

private struct ComposerLinkEditor: View {
    let input: DraftInput
    @State private var title = ""
    @State private var url = ""
    private var valid: Bool {
        guard let value = URL(string: url), let scheme = value.scheme?.lowercased() else { return false }
        return ["https", "http"].contains(scheme) ? value.host != nil : scheme == "mailto" && !value.path.isEmpty
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(L("ui.formatting_toolbar.insert_link")).font(.figtree(14, .semibold))
            TextField(L("ui.formatting_toolbar.text_to_display"), text: $title).textFieldStyle(.roundedBorder)
            TextField("https://…", text: $url).textFieldStyle(.roundedBorder).accessibilityLabel(L("ui.formatting_toolbar.link_address"))
            HStack {
                Spacer()
                Button(L("ui.formatting_toolbar.cancel")) { input.linkRequested = false }.buttonStyle(OutlineButtonStyle())
                Button(L("ui.formatting_toolbar.insert")) { input.linkRequested = false; input.insertLink(title: title, url: url) }
                    .buttonStyle(PrimaryButtonStyle()).disabled(!valid)
            }
        }
        .padding(14).frame(width: 310)
        .onAppear { title = input.selectedText; url = input.currentFormat.link ?? "" }
    }
}
