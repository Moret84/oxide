import AppKit
import XCTest
@testable import Oxide

@MainActor
final class FeedSelectionTests: XCTestCase {
    private let selection = FeedSelection()

    override func setUp() {
        super.setUp()
        let texts: [[(FeedSelection.Unit, NSAttributedString)]] = [
            [(unit("m1", FeedSelection.Unit.authorLine), text("Ann · 10:00")), (unit("m1", 0), text("hello world"))],
            [(unit("m2", FeedSelection.Unit.authorLine), text("Bob · 10:01")), (unit("m2", 0), text("second")),
             (unit("m2", 2), text("third"))],
        ]
        selection.rows = { ["m1": 0, "m2": 1] }
        selection.units = { texts[$0] }
    }

    func testDragAcrossMessagesCopiesTheirAuthorsAndTexts() {
        selection.begin(at: point("m1", 0, 6))
        selection.extend(to: point("m2", 0, 3))

        XCTAssertTrue(selection.spansUnits)
        XCTAssertEqual(selection.text(), "world\n\nBob · 10:01\nsec")
        XCTAssertNil(selection.range(for: unit("m1", FeedSelection.Unit.authorLine), length: 11))
        XCTAssertEqual(selection.range(for: unit("m1", 0), length: 11), NSRange(location: 6, length: 5))
        XCTAssertEqual(selection.range(for: unit("m2", FeedSelection.Unit.authorLine), length: 11), NSRange(location: 0, length: 11))
        XCTAssertNil(selection.range(for: unit("m2", 2), length: 5))
    }

    func testDragUpwardsSelectsTheSame() {
        selection.begin(at: point("m2", 0, 3))
        selection.extend(to: point("m1", 0, 6))

        XCTAssertEqual(selection.text(), "world\n\nBob · 10:01\nsec")
    }

    func testDragWithinOneTextLeavesItToTheTextView() {
        selection.begin(at: point("m1", 0, 0))
        selection.extend(to: point("m1", 0, 5))

        XCTAssertFalse(selection.spansUnits)
    }

    func testClearEndsTheSelection() {
        selection.begin(at: point("m1", 0, 6))
        selection.extend(to: point("m2", 0, 3))
        selection.clear()

        XCTAssertFalse(selection.spansUnits)
        XCTAssertEqual(selection.text(), "")
    }

    private func unit(_ message: String, _ segment: Int) -> FeedSelection.Unit {
        FeedSelection.Unit(message: message, segment: segment)
    }

    private func point(_ message: String, _ segment: Int, _ index: Int) -> FeedSelection.Point {
        FeedSelection.Point(unit: unit(message, segment), index: index)
    }

    private func text(_ string: String) -> NSAttributedString {
        NSAttributedString(string: string)
    }
}
