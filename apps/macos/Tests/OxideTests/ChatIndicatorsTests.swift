import OxideCore
import XCTest
@testable import Oxide

final class ChatIndicatorsTests: XCTestCase {
    func testTypingDotsBounceInTurnAndRestOtherwise() {
        XCTAssertEqual(TypingDots.offset(time: 0, index: 0), 0, accuracy: 0.001)
        XCTAssertEqual(TypingDots.offset(time: 0.2, index: 0), -3, accuracy: 0.001)
        XCTAssertEqual(TypingDots.offset(time: 0.2, index: 1), -CGFloat(3 * sin(0.05 / 0.4 * .pi)), accuracy: 0.001)
        XCTAssertEqual(TypingDots.offset(time: 0.9, index: 0), 0, "rests for the rest of the cycle")
        XCTAssertEqual(TypingDots.offset(time: 1.4, index: 0), TypingDots.offset(time: 0.2, index: 0), accuracy: 0.001)
    }

    func testReceiptsUseACheckWhenSentAndAnEyeOnceRead() {
        XCTAssertEqual(ReceiptIcon.symbol(.sent), "checkmark.circle")
        XCTAssertEqual(ReceiptIcon.symbol(.seen), "eye")
        XCTAssertEqual(ReceiptIcon.symbol(.seenByEveryone), "eye")
        XCTAssertEqual(ReceiptIcon.label(.seenByEveryone), "Vu par tout le monde")
    }
}
