import AppKit
import OxideCore
import UserNotifications
import XCTest
@testable import Oxide

final class NotificationsTests: XCTestCase {
    func testDockBadgeAddsChatsAndActivityAndCapsAtNinetyNine() {
        XCTAssertNil(DockBadge.label(chats: 0, activity: 0))
        XCTAssertEqual(DockBadge.label(chats: 2, activity: 3), "5")
        XCTAssertEqual(DockBadge.label(chats: 99, activity: 1), "99+")
    }

    func testEveryTeamsSoundHasAnInstalledSystemSound() {
        for sound in NotificationSounds.standard + NotificationSounds.urgent {
            let name = NotificationSounds.systemName(sound)
            XCTAssertNotNil(name.flatMap { NSSound(named: NSSound.Name($0)) }, "\(sound)")
            XCTAssertFalse(NotificationSounds.label(sound).isEmpty)
        }
        XCTAssertEqual(Set(NotificationSounds.standard + NotificationSounds.urgent).count, 12)
    }

    func testSystemAuthorizationMapsToTheCorePermission() {
        XCTAssertEqual(NotificationPermissionState.from(.authorized), .granted)
        XCTAssertEqual(NotificationPermissionState.from(.provisional), .granted)
        XCTAssertEqual(NotificationPermissionState.from(.denied), .denied)
        XCTAssertEqual(NotificationPermissionState.from(.notDetermined), .unknown)
    }
}
