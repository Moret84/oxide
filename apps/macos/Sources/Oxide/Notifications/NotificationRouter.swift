import AppKit
import OxideCore
import UserNotifications

/// The only notification center delegate (messages and calls), set at launch so a click on an older banner still arrives.
@MainActor
final class NotificationRouter: NSObject, UNUserNotificationCenterDelegate {
    static let shared = NotificationRouter()

    static let callCategory = "INCOMING_CALL"
    static let callAccept = "ACCEPT"
    static let callDecline = "DECLINE"
    static let messageCategory = "MESSAGE"
    static let messageReplyCategory = "MESSAGE_REPLY"
    static let reply = "REPLY"
    static let markRead = "MARK_READ"

    weak var store: Store? {
        didSet {
            // A click that launched the app arrives before the store exists.
            if let store, let (conversation, chain) = pendingOpen {
                pendingOpen = nil
                store.send(.notifications(action: .open(conversationId: conversation, replyChainId: chain)))
            }
        }
    }
    private var pendingOpen: (String, String?)?
    private var configured = false
    /// Language the notification categories were registered in.
    private var categoriesEnglish: Bool?
    /// Sound of the latest banner, kept until it finishes.
    private var playing: NSSound?

    /// Notifications need a real app bundle (not `swift run`).
    var center: UNUserNotificationCenter? {
        Bundle.main.bundleIdentifier == nil ? nil : UNUserNotificationCenter.current()
    }

    func configure() {
        guard let center else { return }
        if !configured {
            configured = true
            center.delegate = self
        }
        // The action titles follow the app language: registered again when it changes.
        guard categoriesEnglish != L10n.english else { return }
        categoriesEnglish = L10n.english
        let accept = UNNotificationAction(identifier: Self.callAccept, title: L("ui.notification_router.accept"), options: [.foreground])
        let decline = UNNotificationAction(identifier: Self.callDecline, title: L("ui.notification_router.decline"), options: [.destructive])
        let reply = UNTextInputNotificationAction(
            identifier: Self.reply, title: L("ui.notification_router.reply"), options: [],
            textInputButtonTitle: L("ui.notification_router.send"), textInputPlaceholder: L("ui.notification_router.your_reply")
        )
        let read = UNNotificationAction(identifier: Self.markRead, title: L("ui.notification_router.mark_as_read"), options: [])
        center.setNotificationCategories([
            UNNotificationCategory(identifier: Self.callCategory, actions: [accept, decline], intentIdentifiers: []),
            UNNotificationCategory(identifier: Self.messageReplyCategory, actions: [reply, read], intentIdentifiers: []),
            UNNotificationCategory(identifier: Self.messageCategory, actions: [read], intentIdentifiers: []),
        ])
    }

    /// Asks once (the system remembers the answer), then reports the state to the core.
    func requestPermission() {
        guard let center else { return }
        configure()
        center.requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in
            Task { @MainActor in self.refreshPermission() }
        }
    }

    /// The user may change it in System Settings at any time: read it again on activation.
    func refreshPermission() {
        guard let center else {
            store?.send(.notifications(action: .permissionChanged(permission: .unknown)))
            return
        }
        center.getNotificationSettings { settings in
            let permission = NotificationPermissionState.from(settings.authorizationStatus)
            Task { @MainActor in
                self.store?.send(.notifications(action: .permissionChanged(permission: permission)))
            }
        }
    }

    // MARK: Messages

    func show(id: String, conversationId: String, replyChainId: String?, title: String,
              subtitle: String?, body: String, sound: NotificationSound?, urgent: Bool, canReply: Bool) {
        if let sound { play(sound) }
        guard let center else { return }
        configure()
        let content = UNMutableNotificationContent()
        content.title = title
        if let subtitle { content.subtitle = subtitle }
        content.body = body
        content.threadIdentifier = conversationId
        content.categoryIdentifier = canReply ? Self.messageReplyCategory : Self.messageCategory
        content.userInfo = ["conversation": conversationId, "replyChain": replyChainId ?? ""]
        content.interruptionLevel = urgent ? .timeSensitive : .active
        // The sound is played here, on the chosen speaker, so the banner itself is silent.
        content.sound = nil
        center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil))
        if !NSApp.isActive { NSApp.requestUserAttention(urgent ? .criticalRequest : .informationalRequest) }
    }

    /// Removes the delivered banners of a conversation that has been read.
    func withdraw(conversationId: String) {
        guard let center else { return }
        Task {
            let ids = await center.deliveredNotifications()
                .filter { $0.request.content.threadIdentifier == conversationId }
                .map(\.request.identifier)
            if !ids.isEmpty { center.removeDeliveredNotifications(withIdentifiers: ids) }
        }
    }

    func play(_ sound: NotificationSound) {
        let made = NotificationSounds.systemName(sound).flatMap { NSSound(named: NSSound.Name($0)) }
        guard let made else {
            NSSound.beep()
            return
        }
        playing?.stop()
        playing = made
        made.playOnChosenSpeaker()
    }

    // MARK: Calls

    func showCall(id: String, title: String, video: Bool) {
        guard let center else { return }
        configure()
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = video ? L("ui.notification_router.incoming_video_call") : L("ui.notification_router.incoming_audio_call")
        content.categoryIdentifier = Self.callCategory
        content.interruptionLevel = .timeSensitive
        center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil))
    }

    func removeCall(id: String) {
        center?.removeDeliveredNotifications(withIdentifiers: [id])
        center?.removePendingNotificationRequests(withIdentifiers: [id])
    }

    // MARK: Delegate

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            didReceive response: UNNotificationResponse) async {
        let action = response.actionIdentifier
        let request = response.notification.request
        let category = request.content.categoryIdentifier
        let conversation = request.content.userInfo["conversation"] as? String ?? ""
        let chain = (request.content.userInfo["replyChain"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        let text = (response as? UNTextInputNotificationResponse)?.userText
        await MainActor.run {
            guard let store = self.store else {
                if category != Self.callCategory, action == UNNotificationDefaultActionIdentifier {
                    self.pendingOpen = (conversation, chain)
                }
                return
            }
            if category == Self.callCategory {
                guard store.call?.active?.id == request.identifier else { return }
                switch action {
                case Self.callDecline: store.send(.call(action: .decline))
                case Self.callAccept: store.send(.call(action: .accept(video: false)))
                default: NSApp.activate()
                }
                return
            }
            switch action {
            case Self.reply:
                store.send(.notifications(action: .reply(
                    conversationId: conversation, replyChainId: chain, text: text ?? ""
                )))
            case Self.markRead:
                store.send(.notifications(action: .markRead(conversationId: conversation)))
            case UNNotificationDismissActionIdentifier:
                break
            default:
                NSApp.activate()
                store.send(.notifications(action: .open(conversationId: conversation, replyChainId: chain)))
            }
        }
    }

    /// The core already left out banners for the conversation on screen.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }
}

enum NotificationPermissionState {
    static func from(_ status: UNAuthorizationStatus) -> NotificationPermission {
        switch status {
        case .authorized, .provisional, .ephemeral: .granted
        case .denied: .denied
        default: .unknown
        }
    }
    /// System Settings › Notifications, on this app when possible.
    static var settingsURL: URL {
        URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id="
            + (Bundle.main.bundleIdentifier ?? ""))!
    }
}

/// Teams sounds are not shipped with macOS: each gets the closest system sound.
enum NotificationSounds {
    static func systemName(_ sound: NotificationSound) -> String? {
        switch sound {
        case .default: "Glass"
        case .vibe: "Purr"
        case .nudge: "Pop"
        case .tap: "Tink"
        case .flick: "Morse"
        case .pluck: "Bottle"
        case .summon: "Hero"
        case .ping: "Ping"
        case .defaultUrgent: "Sosumi"
        case .prioritize: "Blow"
        case .escalate: "Funk"
        case .alarm: "Submarine"
        }
    }
    static func label(_ sound: NotificationSound) -> String {
        switch sound {
        case .default: L("ui.notification_router.default")
        case .vibe: "Vibe"
        case .nudge: L("ui.notification_router.nudge")
        case .tap: L("ui.notification_router.tap")
        case .flick: L("ui.notification_router.flick")
        case .pluck: L("ui.notification_router.pluck")
        case .summon: L("ui.notification_router.summon")
        case .ping: "Ping"
        case .defaultUrgent: L("ui.notification_router.default_urgent")
        case .prioritize: L("ui.notification_router.prioritize")
        case .escalate: L("ui.notification_router.escalate")
        case .alarm: L("ui.notification_router.alarm")
        }
    }
    static let standard: [NotificationSound] = [.default, .vibe, .nudge, .tap, .flick, .pluck, .summon, .ping]
    static let urgent: [NotificationSound] = [.defaultUrgent, .prioritize, .escalate, .alarm]
}

/// Unread chats plus unread activity, on the Dock icon.
enum DockBadge {
    static func label(chats: UInt32, activity: UInt32) -> String? {
        let total = Int(chats) + Int(activity)
        if total <= 0 { return nil }
        return total > 99 ? "99+" : String(total)
    }
}
