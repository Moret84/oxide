import SwiftUI
import Observation
import OxideCore

/// Inline images remain inside a single Text, so wrapping and paragraph layout are native.
struct MessageEmojiText: View {
    let runs: [RunView]
    var size: Double = 14

    var body: some View {
        // Read so the text is split again once the catalog arrives.
        let _ = TeamsEmojiCatalog.shared.revision
        let fragments = MessageEmojiFragment.cached(runs, size: size)
        let ids = Set(fragments.compactMap(\.emojiID))
        fragments.reduce(Text("")) { result, fragment in
            switch fragment {
            case .text(let text):
                return result + Text(text)
            case .emoji(let item, let fallback, let side):
                if let sized = MessageEmojiImage.cached(item.id).sized(side) {
                    return result + Text(Image(nsImage: sized))
                        .baselineOffset(-side * 0.18)
                        .accessibilityLabel(Text(item.text))
                }
                return result + Text(fallback)
            }
        }
        .task(id: ids) {
            // Rows scrolling back into view usually have every image already.
            let missing = ids.filter { MessageEmojiImage.cached($0).image == nil }
            guard !missing.isEmpty else { return }
            await withTaskGroup(of: Void.self) { group in
                for id in missing {
                    group.addTask { await MessageEmojiImage.cached(id).load() }
                }
            }
        }
    }
}

enum MessageEmojiFragment {
    case text(AttributedString)
    case emoji(ComposerEmoji, fallback: AttributedString, size: Double)

    var emojiID: String? {
        if case .emoji(let item, _, _) = self { return item.id }
        return nil
    }

    /// Splitting is redone only for runs not seen recently: a feed redraw reuses every row's fragments.
    @MainActor private static let cache: NSCache<FragmentKey, FragmentBox> = {
        let cache = NSCache<FragmentKey, FragmentBox>()
        cache.countLimit = 2_000
        return cache
    }()

    @MainActor static func cached(_ runs: [RunView], size: Double) -> [Self] {
        let key = FragmentKey(runs: runs, size: size)
        if let box = cache.object(forKey: key) { return box.fragments }
        let fragments = make(runs, size: size)
        cache.setObject(FragmentBox(fragments), forKey: key)
        return fragments
    }

    @MainActor static func clearCache() {
        cache.removeAllObjects()
    }

    @MainActor static func make(_ runs: [RunView], size: Double) -> [Self] {
        var result: [Self] = []
        for run in runs {
            // Keep literal code and clickable emoji links intact.
            guard !run.code, run.link == nil else {
                result.append(.text(MessageText.attributed([run], size: size)))
                continue
            }
            var buffer = ""
            func styled(_ text: String) -> AttributedString {
                var piece = run
                piece.text = text
                return MessageText.attributed([piece], size: size)
            }
            for glyph in run.text {
                if let item = TeamsEmojiCatalog.emoji(for: glyph) {
                    if !buffer.isEmpty {
                        result.append(.text(styled(buffer)))
                        buffer = ""
                    }
                    let fontSize = run.size == 0 ? size : Double(run.size)
                    result.append(.emoji(item, fallback: styled(String(glyph)), size: fontSize * 1.2))
                } else {
                    buffer.append(glyph)
                }
            }
            if !buffer.isEmpty { result.append(.text(styled(buffer))) }
        }
        return result
    }

    /// Hashes the runs once; equality still compares them, so a collision cannot mix up messages.
    private final class FragmentKey: NSObject {
        let runs: [RunView]
        let size: Double
        private let digest: Int
        init(runs: [RunView], size: Double) {
            self.runs = runs
            self.size = size
            var hasher = Hasher()
            hasher.combine(runs)
            hasher.combine(size)
            digest = hasher.finalize()
        }
        override var hash: Int { digest }
        override func isEqual(_ object: Any?) -> Bool {
            guard let other = object as? FragmentKey else { return false }
            return digest == other.digest && size == other.size && runs == other.runs
        }
    }

    private final class FragmentBox {
        let fragments: [MessageEmojiFragment]
        init(_ fragments: [MessageEmojiFragment]) { self.fragments = fragments }
    }
}

/// Each image is observed separately; loading one emoji only redraws messages using it.
@MainActor @Observable
final class MessageEmojiImage {
    /// Bounded; an evicted entry is simply fetched again (URLSession's cache usually answers).
    private static let cache: NSCache<NSString, MessageEmojiImage> = {
        let cache = NSCache<NSString, MessageEmojiImage>()
        cache.countLimit = 1_000
        return cache
    }()
    private(set) var image: NSImage?
    @ObservationIgnored private var request: Task<NSImage?, Never>?
    @ObservationIgnored private let id: String
    /// Inline copies drawn at a given side, so a redraw does not allocate a new NSImage per emoji.
    @ObservationIgnored private var sizedImages: [Double: NSImage] = [:]
    /// A failed download is not retried on every appearance, only after `retryDelay`.
    @ObservationIgnored private var failedAt: Date?
    private static let retryDelay: TimeInterval = 300

    private init(id: String) { self.id = id }

    static func cached(_ id: String) -> MessageEmojiImage {
        if let image = cache.object(forKey: id as NSString) { return image }
        let image = MessageEmojiImage(id: id)
        cache.setObject(image, forKey: id as NSString)
        // A view may still show an evicted entry: its replacement must load without waiting for a new task.
        Task { await image.load() }
        return image
    }

    /// The artwork at `side` points, or nil while it is not loaded.
    func sized(_ side: Double) -> NSImage? {
        guard let picture = image else { return nil }
        if let sized = sizedImages[side] { return sized }
        let sized = NSImage(size: NSSize(width: side, height: side), flipped: false) { rect in
            picture.draw(in: rect)
            return true
        }
        sizedImages[side] = sized
        return sized
    }

    func load() async {
        guard image == nil else { return }
        if let failedAt, Date().timeIntervalSince(failedAt) < Self.retryDelay { return }
        if request == nil {
            let id = id
            request = Task {
                guard let url = TeamsEmojiCatalog.assetURL(id),
                      let (data, response) = try? await URLSession.shared.data(from: url),
                      let response = response as? HTTPURLResponse, response.statusCode == 200 else { return nil }
                return NSImage(data: data)
            }
        }
        if let picture = await request?.value {
            image = picture
            failedAt = nil
        } else {
            failedAt = Date()
        }
        request = nil
    }
}
