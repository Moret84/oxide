import AppKit
import XCTest
import OxideCore
@testable import Oxide

@MainActor
final class MessageRichTextTests: XCTestCase {
    func testTextLikeBlocksMakeOneText() {
        let pieces = MessageRichText.pieces([
            paragraph("a"),
            .list(ordered: false, start: 1, items: [ListItemView(blocks: [paragraph("b")])]),
            .quote(author: nil, time: nil, messageId: nil, body: [paragraph("c")]),
            .table(header: false, rows: [TableRowView(cells: [TableCellView(runs: [run("d")]), TableCellView(runs: [run("e")])])]),
        ], size: 14)

        XCTAssertEqual(pieces.count, 1)
        guard case .text(let text, let spacing) = pieces[0] else { return XCTFail("one text") }
        XCTAssertEqual(text.string, "a\n\t•\tb\nc\nd\ne")
        XCTAssertEqual(spacing, 0)
    }

    func testImageInsideAQuoteCutsItsBoxWhichGoesOn() {
        let pieces = MessageRichText.pieces([
            .quote(author: nil, time: nil, messageId: nil, body: [paragraph("a"), .image(image: image), paragraph("b")]),
        ], size: 14)

        XCTAssertEqual(pieces.count, 3)
        guard case .text(let before, _) = pieces[0], case .other(.image, let placement) = pieces[1],
              case .text(let after, _) = pieces[2] else { return XCTFail("text, image, text") }
        XCTAssertEqual(placement.leading, 12)
        XCTAssertEqual(placement.quote?.top, false)
        XCTAssertEqual(placement.quote?.bottom, false)
        XCTAssertEqual(box(before)?.openBottom, true)
        XCTAssertEqual(box(before)?.openTop, false)
        XCTAssertEqual(box(after)?.openTop, true)
        XCTAssertEqual(box(after)?.openBottom, false)
    }

    func testQuoteHoldingOnlyAnImageRoundsAroundIt() {
        let pieces = MessageRichText.pieces([.quote(author: nil, time: nil, messageId: nil, body: [.image(image: image)])], size: 14)

        XCTAssertEqual(pieces.count, 1)
        guard case .other(.image, let placement) = pieces[0] else { return XCTFail("image") }
        XCTAssertEqual(placement.spacing, 0)
        XCTAssertEqual(placement.quote?.spacing, 6)
        XCTAssertEqual(placement.quote?.top, true)
        XCTAssertEqual(placement.quote?.bottom, true)
    }

    func testCodeStandsApartAndSelectsAsItsText() {
        let pieces = MessageRichText.pieces([paragraph("a"), .code(language: "swift", text: "let x = 1")], size: 14)

        XCTAssertEqual(pieces.count, 2)
        guard case .other(.code, let placement) = pieces[1] else { return XCTFail("code") }
        XCTAssertEqual(placement.spacing, MessageRichText.blockSpacing)
        XCTAssertNil(placement.quote)
        XCTAssertEqual(pieces[1].selectable?.string, "let x = 1")
    }

    func testPlainTextWritesEmojiImagesBackAsEmoji() {
        let text = NSMutableAttributedString(string: "ok ")
        text.append(NSAttributedString(attachment: NSTextAttachment()))
        text.addAttribute(MessageRichText.emoji, value: "👍", range: NSRange(location: 3, length: 1))

        XCTAssertEqual(MessageRichText.plain(text, range: NSRange(location: 0, length: text.length)), "ok 👍")
    }

    func testShortTextKeepsItsWidthAfterBeingMeasuredWider() {
        guard case .text(let text, _) = MessageRichText.pieces([paragraph("pas de souci")], size: 14)[0] else {
            return XCTFail("text")
        }
        let view = MessageTextView()
        view.textStorage?.setAttributedString(text)
        let narrow = view.fit(width: 560)

        XCTAssertLessThan(narrow.width, 200)
        XCTAssertEqual(view.fit(width: 100_000), narrow)
        XCTAssertEqual(view.fit(width: 560), narrow)
    }

    func testQuoteKeepsRoomForItsPadding() {
        let quote = MessageRichText.pieces([.quote(author: nil, time: nil, messageId: nil, body: [paragraph("ok")])], size: 14)
        guard case .text(let text, _) = quote[0] else { return XCTFail("text") }
        let plain = MessageTextView()
        plain.textStorage?.setAttributedString(NSAttributedString(string: "ok", attributes: text.attributes(at: 0, effectiveRange: nil)
            .filter { $0.key != .paragraphStyle }))
        let boxed = MessageTextView()
        boxed.textStorage?.setAttributedString(text)

        XCTAssertEqual(boxed.fit(width: 560).width, plain.fit(width: 560).width + 12 + 10)
    }

    private var image: ImageView {
        ImageView(src: "https://example.test/a.png", alt: "", status: .loading, width: 10, height: 10)
    }

    private func box(_ text: NSAttributedString) -> MessageTextBlock? {
        (text.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle)?
            .textBlocks.first as? MessageTextBlock
    }

    private func paragraph(_ text: String) -> BlockView {
        .text(runs: [run(text)])
    }

    private func run(_ text: String) -> RunView {
        RunView(text: text, link: nil, bold: false, italic: false, code: false,
                underline: false, strike: false, color: nil, highlight: nil, size: 0, mention: false)
    }
}
