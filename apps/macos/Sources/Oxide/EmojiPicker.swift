import SwiftUI
import OxideCore

/// The downloaded Teams catalog indexed by glyph; `revision` lets messages already drawn pick it up.
@MainActor @Observable
final class TeamsEmojiCatalog {
    static let shared = TeamsEmojiCatalog()
    private(set) var revision = 0
    @ObservationIgnored private var byGlyph: [String: ComposerEmoji] = [:]

    static func update(_ categories: [EmojiCategory]) {
        shared.byGlyph = Dictionary(categories.flatMap(\.items).filter { $0.text.count == 1 }.map {
            (normalizedGlyph($0.text), $0)
        }, uniquingKeysWith: { first, _ in first })
        MessageEmojiFragment.clearCache()
        shared.revision += 1
    }

    private static func normalizedGlyph(_ text: String) -> String {
        text.replacingOccurrences(of: "\u{FE0F}", with: "")
    }

    static func emoji(for glyph: Character) -> ComposerEmoji? {
        guard glyph.unicodeScalars.contains(where: { $0.properties.isEmoji }),
              glyph.unicodeScalars.contains(where: { $0.value > 127 }) else { return nil }
        return shared.byGlyph[normalizedGlyph(String(glyph))]
    }

    nonisolated static func assetURL(_ id: String) -> URL? {
        URL(string: "https://teams.public.onecdn.static.microsoft/evergreen-assets/personal-expressions/v2/assets/emoticons/\(id)/default/40_f.png")
    }
}

/// Teams artwork (from the shared inline cache), with the native glyph while it loads or is unavailable.
struct TeamsEmoji: View {
    let id: String?
    let text: String
    let size: CGFloat

    var body: some View {
        let artwork = id.map(MessageEmojiImage.cached)
        Group {
            if let image = artwork?.image {
                Image(nsImage: image).resizable().scaledToFit()
            } else {
                Text(text).font(.system(size: size * 24 / 28))
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel(text)
        .task(id: id) { await artwork?.load() }
    }
}

/// Teams' personal-expressions catalog in the app's language, with its IDs and search vocabulary.
struct EmojiPicker: View {
    let categories: [EmojiCategory]
    let onChoose: (ComposerEmoji) -> Void
    @State private var search = ""
    @State private var category = 0
    @AppStorage("composerRecentEmoji") private var recent = ""
    /// Every emoji with its folded search text, built once per catalog rather than per keystroke.
    @State private var index: [(item: ComposerEmoji, key: String)] = []
    private let icons = ["🙂", "👋", "👤", "🐵", "🍇", "🌍", "👓", "🎃", "🔣"]

    /// Case-, accent- and width-insensitive, like `localizedStandardContains`.
    static func searchKey(_ text: String) -> String {
        text.folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], locale: .current)
    }
    private static func makeIndex(_ categories: [EmojiCategory]) -> [(item: ComposerEmoji, key: String)] {
        categories.flatMap(\.items).map {
            ($0, searchKey($0.name + " " + $0.keywords + " " + $0.id + " " + $0.text))
        }
    }
    private func results() -> [ComposerEmoji] {
        let query = search.trimmingCharacters(in: .whitespacesAndNewlines)
        if query.isEmpty { return categories.indices.contains(category) ? categories[category].items : [] }
        let folded = Self.searchKey(query)
        return index.filter { $0.key.contains(folded) }.map(\.item)
    }
    private func recents() -> [ComposerEmoji] {
        let ids = recent.split(separator: "|").map(String.init)
        guard !ids.isEmpty else { return [] }
        let byID = Dictionary(categories.flatMap(\.items).map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        return ids.compactMap { byID[$0] }
    }
    private func icon(_ i: Int) -> String { icons.indices.contains(i) ? icons[i] : "•" }
    var body: some View {
        let results = results()
        let recents = search.isEmpty ? recents() : []
        VStack(alignment: .leading, spacing: 10) {
            Text(L("ui.emoji_picker.emoji")).font(.figtree(15, .semibold))
            TextField(L("ui.emoji_picker.search_emoji"), text: $search)
                .textFieldStyle(.roundedBorder).accessibilityLabel(L("ui.emoji_picker.search_emoji"))
            HStack(spacing: 1) {
                ForEach(Array(categories.enumerated()), id: \.element.id) { i, item in
                    Button { category = i; search = "" } label: {
                        Text(icon(i)).font(.system(size: 20)).frame(width: 34, height: 32)
                            .background(category == i ? Theme.accent.opacity(0.15) : .clear, in: RoundedRectangle(cornerRadius: 6))
                    }
                    .buttonStyle(.plain).pointerCursor().help(item.name).accessibilityLabel(item.name)
                }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    if search.isEmpty && !recents.isEmpty {
                        Text(L("ui.emoji_picker.recent")).font(.figtree(12, .semibold)).foregroundStyle(Theme.muted)
                        grid(recents)
                    }
                    Text(search.isEmpty ? (categories.indices.contains(category) ? categories[category].name : L("ui.emoji_picker.emoji"))
                         : L("ui.emoji_picker.results", ["resultsCount": results.count]))
                        .font(.figtree(12, .semibold)).foregroundStyle(Theme.muted)
                    if categories.isEmpty {
                        Text(L("ui.emoji_picker.downloading")).font(.figtree(13)).foregroundStyle(Theme.muted)
                    } else if results.isEmpty { Text(L("ui.emoji_picker.no_emoji_found")).font(.figtree(13)).foregroundStyle(Theme.muted) }
                    grid(results)
                }.padding(.trailing, 4)
            }.frame(height: 260)
        }.padding(12).frame(width: 340)
            .onChange(of: categories, initial: true) { _, categories in index = Self.makeIndex(categories) }
    }
    private func grid(_ items: [ComposerEmoji]) -> some View {
        LazyVGrid(columns: Array(repeating: GridItem(.fixed(36), spacing: 3), count: 8), spacing: 3) {
            ForEach(items, id: \.id) { item in
                Button {
                    var ids = recent.split(separator: "|").map(String.init).filter { $0 != item.id }
                    ids.insert(item.id, at: 0)
                    recent = ids.prefix(24).joined(separator: "|")
                    onChoose(item)
                } label: {
                    TeamsEmoji(id: item.id, text: item.text, size: 28)
                        .frame(width: 36, height: 36)
                }
                .buttonStyle(GhostButtonStyle()).help(item.name).accessibilityLabel(item.name)
            }
        }
    }
}
