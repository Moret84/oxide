import SwiftUI
import OxideCore

extension SearchFiltersView {
    static let empty = SearchFiltersView(from: nil, date: .any, start: "", end: "", kind: .everywhere,
                                         hasAttachment: false, mentionsMe: false, hideBots: false, here: false)
}

/// Messages found on the server, with the web client's filters; a result opens its chat, channel or thread.
struct SearchResultsScreen: View {
    @Environment(Store.self) private var store
    @State private var person = ""
    var body: some View {
        if let view = store.search {
            VStack(alignment: .leading, spacing: 0) {
                header(view)
                Divider().overlay(Theme.border)
                filters(view)
                Divider().overlay(Theme.border)
                results(view)
            }
        }
    }

    private func run(_ view: SearchView, _ change: (inout SearchFiltersView) -> Void) {
        var filters = view.filters
        change(&filters)
        store.send(.fullSearch(action: .run(query: view.query, filters: filters)))
    }

    /// A filter that searches again whenever it changes.
    private func filter<Value>(_ view: SearchView, _ key: WritableKeyPath<SearchFiltersView, Value>) -> Binding<Value> {
        Binding(get: { view.filters[keyPath: key] }, set: { value in run(view) { $0[keyPath: key] = value } })
    }

    private func header(_ view: SearchView) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(L("ui.search.title")).font(.figtree(20, .semibold))
                if view.searched {
                    Muted(L("ui.search.results_count", ["count": view.total]), size: 12)
                }
            }
            Spacer()
            IconButton(symbol: "xmark", help: L("ui.search.close")) { store.send(.fullSearch(action: .close)) }
                .keyboardShortcut(.cancelAction)
        }
        .padding(.horizontal, 20).padding(.vertical, 12)
    }

    private func filters(_ view: SearchView) -> some View {
        let f = view.filters
        return ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 10) {
                Menu {
                    Button(L("ui.search.anyone")) { run(view) { $0.from = nil } }
                    Divider()
                    ForEach(view.people, id: \.mri) { p in
                        Button(p.email.isEmpty ? p.name : "\(p.name) · \(p.email)") { run(view) { $0.from = p } }
                    }
                } label: {
                    Label(f.from.map { "\(L("ui.search.from")) : \($0.name)" } ?? L("ui.search.from"), systemImage: "person")
                }
                .fixedSize()
                TextField(L("ui.search.person"), text: $person)
                    .textFieldStyle(.roundedBorder).frame(width: 150)
                    .task(id: person) {
                        try? await Task.sleep(for: .milliseconds(300))
                        guard !Task.isCancelled else { return }
                        store.send(.fullSearch(action: .findPeople(query: person)))
                    }
                Picker(L("ui.search.date"), selection: filter(view, \.date)) {
                    Text(L("ui.search.any_time")).tag(SearchDate.any)
                    Text(L("ui.search.today")).tag(SearchDate.today)
                    Text(L("ui.search.yesterday")).tag(SearchDate.yesterday)
                    Text(L("ui.search.this_week")).tag(SearchDate.thisWeek)
                    Text(L("ui.search.last_week")).tag(SearchDate.lastWeek)
                    Text(L("ui.search.this_month")).tag(SearchDate.thisMonth)
                    Text(L("ui.search.last_month")).tag(SearchDate.lastMonth)
                    Text(L("ui.search.this_year")).tag(SearchDate.thisYear)
                    Text(L("ui.search.last_year")).tag(SearchDate.lastYear)
                    Text(L("ui.search.custom_range")).tag(SearchDate.custom)
                }
                .fixedSize()
                if f.date == .custom {
                    DatePicker("", selection: day(view, start: true), displayedComponents: .date).labelsHidden()
                    Text("–")
                    DatePicker("", selection: day(view, start: false), displayedComponents: .date).labelsHidden()
                }
                Picker(L("ui.search.type"), selection: filter(view, \.kind)) {
                    Text(L("ui.search.everywhere")).tag(SearchKind.everywhere)
                    Text(L("ui.search.chats")).tag(SearchKind.chats)
                    Text(L("ui.search.channels")).tag(SearchKind.channels)
                }
                .fixedSize()
                Toggle(L("ui.search.has_attachment"), isOn: filter(view, \.hasAttachment))
                Toggle(L("ui.search.mentions_me"), isOn: filter(view, \.mentionsMe))
                Toggle(L("ui.search.hide_bots"), isOn: filter(view, \.hideBots))
                if let here = view.hereName {
                    Toggle(L("ui.search.only_in", ["name": here]), isOn: filter(view, \.here))
                }
            }
            .toggleStyle(.checkbox)
            .font(.figtree(12))
            .padding(.horizontal, 20).padding(.vertical, 10)
        }
    }

    private static let dayFormat: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd"
        return f
    }()

    private func day(_ view: SearchView, start: Bool) -> Binding<Date> {
        let f = Self.dayFormat
        let value = start ? view.filters.start : view.filters.end
        return Binding(
            get: { f.date(from: value) ?? Date() },
            set: { date in
                run(view) {
                    let text = f.string(from: date)
                    if start { $0.start = text } else { $0.end = text }
                    if $0.start.isEmpty { $0.start = text }
                    if $0.end.isEmpty { $0.end = text }
                }
            }
        )
    }

    private func results(_ view: SearchView) -> some View {
        let terms = Self.terms(view.terms + view.query.split(separator: " ").map(String.init))
        return ScrollView {
            LazyVStack(alignment: .leading, spacing: 8) {
                if let error = view.error {
                    Text(error).font(.figtree(12)).foregroundStyle(Theme.errorText).textSelection(.enabled)
                }
                ForEach(view.hits, id: \.messageId) { hit in
                    Button { store.send(.fullSearch(action: .openHit(messageId: hit.messageId))) } label: {
                        HStack(alignment: .top, spacing: 12) {
                            Avatar(name: hit.sender, id: hit.senderId, size: 32)
                            VStack(alignment: .leading, spacing: 4) {
                                HStack {
                                    Text(hit.sender).font(.figtree(13, .semibold))
                                    Spacer()
                                    Muted(hit.time, size: 11)
                                }
                                Text(highlighted(hit.preview, terms: terms))
                                    .font(.figtree(13)).lineLimit(3)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                Label(hit.place, systemImage: hit.isChannel ? "number" : "bubble.left.and.bubble.right")
                                    .font(.figtree(11)).foregroundStyle(Theme.muted)
                            }
                        }
                        .padding(12).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain).pointerCursor()
                    .card()
                }
                if view.loading {
                    LoadingLabel(text: L("ui.search.searching")).padding()
                } else if view.searched && view.hits.isEmpty && view.error == nil {
                    Muted(L("ui.search.no_results"), size: 13).padding()
                } else if !view.searched {
                    Muted(L("ui.search.type_or_filter"), size: 13).padding()
                }
                if view.hasMore && !view.loading {
                    Button(L("ui.search.more_results")) { store.send(.fullSearch(action: .more)) }
                        .buttonStyle(GhostButtonStyle(accent: true)).padding(.vertical, 6)
                }
            }
            .padding(20)
            .frame(maxWidth: 820, alignment: .leading)
        }
    }

    /// The matched words in bold, as the web client highlights them from `SearchTerms`.
    /// The words to highlight, worked out once for all hits.
    private static func terms(_ words: [String]) -> Set<String> {
        Set(words.map { $0.trimmingCharacters(in: .punctuationCharacters) }.filter { $0.count > 1 })
    }

    private func highlighted(_ text: String, terms: Set<String>) -> AttributedString {
        var result = AttributedString(text)
        for term in terms {
            var start = result.startIndex
            while let range = result[start...].range(of: term, options: [.caseInsensitive, .diacriticInsensitive]) {
                result[range].inlinePresentationIntent = .stronglyEmphasized
                result[range].foregroundColor = Theme.accentText
                start = range.upperBound
            }
        }
        return result
    }
}
