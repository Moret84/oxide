import XCTest
import OxideCore
@testable import Oxide

@MainActor
final class EmojiTests: XCTestCase {
    /// The real catalog is downloaded at launch; tests use a few entries in its shape.
    override func setUp() {
        super.setUp()
        let items = [("yes", "👍"), ("heart", "❤️"), ("1f44d_thumbsup-tone3", "👍🏽")].map {
            ComposerEmoji(id: $0.0, name: $0.0, text: $0.1, keywords: "")
        }
        TeamsEmojiCatalog.update([EmojiCategory(id: "test", name: "Test", items: items)])
    }

    func testEmptyCatalogKeepsNativeGlyphs() {
        TeamsEmojiCatalog.update([])
        XCTAssertNil(TeamsEmojiCatalog.emoji(for: "👍"))
        XCTAssertTrue(MessageEmojiFragment.make([run("Bravo 👍")], size: 14).compactMap(\.emojiID).isEmpty)
    }

    private func run(_ text: String, code: Bool = false, link: String? = nil) -> RunView {
        RunView(text: text, link: link, bold: true, italic: false, code: code,
                underline: false, strike: false, color: nil, highlight: nil, size: 0, mention: false)
    }

    func testConsecutiveParagraphsAreJoinedSoASelectionSpansThem() {
        let code = BlockView.code(language: nil, text: "x")
        let joined = MessageBlocks.joiningParagraphs([.text(runs: [run("a")]), .text(runs: [run("b")]), code, .text(runs: [run("c")])])
        XCTAssertEqual(joined.count, 3)
        guard case .text(let runs) = joined[0] else { return XCTFail("paragraphs") }
        XCTAssertEqual(runs.map(\.text).joined(), "a\n\nb")
        XCTAssertEqual(joined[1], code)
    }

    func testInlineArtworkKeepsSurroundingTextAndNativeFallback() {
        let source = run("Bravo 👍 ! ❤️\nEncore")
        let fragments = MessageEmojiFragment.make([source], size: 14)
        let reconstructed = fragments.map { fragment -> String in
            switch fragment {
            case .text(let text): return String(text.characters)
            case .emoji(_, let fallback, _): return String(fallback.characters)
            }
        }.joined()
        XCTAssertEqual(reconstructed, source.text)
        XCTAssertEqual(fragments.compactMap(\.emojiID).count, 2)
        guard case .text(let first) = fragments.first else { return XCTFail("Missing leading text") }
        XCTAssertEqual(first, MessageText.attributed([run("Bravo ")]))
    }

    func testInlineArtworkPreservesGraphemesCodeAndLinks() {
        XCTAssertEqual(TeamsEmojiCatalog.emoji(for: "❤")?.id, TeamsEmojiCatalog.emoji(for: "❤️")?.id)
        XCTAssertNil(TeamsEmojiCatalog.emoji(for: "1"))
        for glyph: Character in ["👍🏽", "👨‍👩‍👧‍👦", "🇫🇷"] {
            if let item = TeamsEmojiCatalog.emoji(for: glyph) {
                XCTAssertEqual(item.text, String(glyph))
            }
        }
        let code = run("👍", code: true)
        let link = run("❤️", link: "https://example.com")
        let fragments = MessageEmojiFragment.make([code, link], size: 14)
        XCTAssertTrue(fragments.compactMap(\.emojiID).isEmpty)
        guard case .text(let linked) = fragments.last else { return XCTFail("Missing link") }
        XCTAssertEqual(linked, MessageText.attributed([link]))
    }

    func testEmojiOnlyMessagesKeepTheirLargerArtwork() {
        let fragments = MessageEmojiFragment.make([run("👍")], size: 30)
        guard case .emoji(_, _, let size) = fragments.first else { return XCTFail("Missing artwork") }
        XCTAssertEqual(size, 36)
    }
}
