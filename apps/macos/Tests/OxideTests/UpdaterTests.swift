import XCTest
@testable import Oxide

@MainActor
final class UpdaterTests: XCTestCase {
    func testVersionOrdering() {
        XCTAssertTrue(Updater.isNewer("0.1.1", than: "0.1.0"))
        XCTAssertTrue(Updater.isNewer("v0.2.0", than: "0.1.9"))
        XCTAssertTrue(Updater.isNewer("0.10.0", than: "0.9.1"))
        XCTAssertTrue(Updater.isNewer("1.0", than: "0.9.9"))
        XCTAssertFalse(Updater.isNewer("0.1.0", than: "0.1.0"))
        XCTAssertFalse(Updater.isNewer("v0.1", than: "0.1.0"))
        XCTAssertFalse(Updater.isNewer("0.1.0", than: "0.2.0"))
        XCTAssertFalse(Updater.isNewer("0.2.0-beta", than: "0.2.0"))
    }

    /// The shape of `releases/latest` for v0.1.0, trimmed to the fields read.
    func testDecodesRelease() throws {
        let json = """
        {"tag_name":"v0.1.0","html_url":"https://github.com/neicureuil/oxide/releases/tag/v0.1.0",
         "body":"## Installation\\r\\n\\r\\n1. Download **it**.",
         "assets":[{"name":"notes.txt","browser_download_url":"https://example.com/notes.txt","digest":null},
                   {"name":"Oxide-0.1.0.zip",
                    "browser_download_url":"https://github.com/neicureuil/oxide/releases/download/v0.1.0/Oxide-0.1.0.zip",
                    "digest":"sha256:de09de601a49e70e35a164d907985df947ac0fe9688720f4515303c392a19575"}]}
        """
        let release = try Updater.decode(Data(json.utf8))
        XCTAssertEqual(release.version, "0.1.0")
        XCTAssertEqual(release.archive?.name, "Oxide-0.1.0.zip")
        XCTAssertEqual(release.archive?.digest?.hasPrefix("sha256:"), true)
        XCTAssertEqual(String(release.notes.characters), "Installation\n\n1. Download it.")
    }
}
