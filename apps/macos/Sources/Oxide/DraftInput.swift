import AppKit
import Observation
import OxideCore

struct MentionQuery: Equatable {
    let query: String
    let range: NSRange

    static func detect(_ value: String, selection: NSRange) -> MentionQuery? {
        guard selection.length == 0, selection.location <= (value as NSString).length,
              let caret = Range(selection, in: value)?.lowerBound else { return nil }
        let prefix = value[..<caret]
        guard let at = prefix.lastIndex(of: "@"),
              at == value.startIndex || value[value.index(before: at)].isWhitespace || "([{>".contains(value[value.index(before: at)]) else { return nil }
        let query = String(prefix[value.index(after: at)...])
        guard query.count <= 64, !query.contains(where: { $0.isNewline || $0 == "@" }) else { return nil }
        return MentionQuery(query: query, range: NSRange(at..<caret, in: value))
    }
}

@Observable
final class DraftInput {
    @ObservationIgnored weak var textView: NSTextView?
    var currentFormat = ComposerText.empty
    var linkRequested = false
    var mention: MentionQuery?
    var selection = NSRange(location: 0, length: 0)
    var highlighted = 0
    private var suppressed: MentionQuery?
    private var value = ""
    private var skipCoreMention = false

    func dismiss(afterInsertion: Bool = false) {
        suppressed = mention
        mention = nil
        skipCoreMention = afterInsertion
    }
    func capture(_ value: String, selection: NSRange, mentions: [MentionRange] = [], coreChange: Bool = false) {
        self.selection = selection
        refreshFormat()
        let detected = MentionQuery.detect(value, selection: selection)
        let found = detected.flatMap { query in
            mentions.contains { NSIntersectionRange(query.range, NSRange(location: Int($0.start), length: Int($0.length))).length > 0 } ? nil : query
        }
        if coreChange && skipCoreMention {
            self.value = value
            suppressed = found
            mention = nil
            skipCoreMention = false
            return
        }
        if self.value != value { suppressed = nil; if !coreChange { skipCoreMention = false } }
        self.value = value
        mention = found == suppressed ? nil : found
    }
}
