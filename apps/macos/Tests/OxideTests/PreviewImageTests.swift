import OxideCore
import XCTest
@testable import Oxide

final class PreviewImageTests: XCTestCase {
    private func image(width: UInt32?, height: UInt32?) -> ImageView {
        ImageView(src: "https://example.test/a.png", alt: "", status: .loading, width: width, height: height)
    }

    /// A card badge declaring only `"height": "24px"` stays 24 points high instead of taking the default box.
    func testAnImageWithOnlyAHeightKeepsThatHeight() {
        XCTAssertEqual(PreviewImage.declaredHeight(image(width: nil, height: 24)), 24)
        XCTAssertEqual(PreviewImage.estimatedHeight(image(width: nil, height: 24), width: 500), 24)
    }

    /// Its width is its own, so an `auto` column holding it leaves room for the columns beside it.
    func testAnImageWithOnlyAHeightTakesThePictureAspectRatio() {
        XCTAssertEqual(PreviewImage.width(atHeight: 24, of: nil), 24)
        XCTAssertEqual(PreviewImage.width(atHeight: 24, of: NSImage(size: NSSize(width: 150, height: 48))), 75)
        XCTAssertEqual(PreviewImage.width(atHeight: 24, of: NSImage(size: NSSize(width: 10_000, height: 10))), PreviewImage.maxSize.width)
    }

    func testDeclaredSizesAndUnknownSizesAreUnchanged() {
        XCTAssertNil(PreviewImage.declaredHeight(image(width: 40, height: 40)))
        XCTAssertEqual(PreviewImage.estimatedHeight(image(width: 400, height: 200), width: 200), 100)
        XCTAssertEqual(PreviewImage.estimatedHeight(image(width: nil, height: nil), width: 500), 250)
    }
}
