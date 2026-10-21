import SwiftUI
import OxideCore

/// How conversations stay current, kept in the user defaults and handed to the core.
enum RefreshPreference: String, CaseIterable, Identifiable {
    case push, polling

    static let storageKey = "refresh.mode"

    static var stored: Self {
        UserDefaults.standard.string(forKey: storageKey).flatMap(Self.init(rawValue:)) ?? .push
    }

    var id: Self { self }
    var label: String {
        switch self {
        case .push: L("ui.refresh_settings.real_time")
        case .polling: L("ui.refresh_settings.constant_refresh")
        }
    }
    var detail: String {
        switch self {
        case .push: L("ui.refresh_settings.push_detail")
        case .polling: L("ui.refresh_settings.polling_detail")
        }
    }
    var core: RefreshMode {
        switch self {
        case .push: .push
        case .polling: .polling
        }
    }
}

/// Settings › Actualisation: push only, or polling all the time.
struct RefreshSettingsSection: View {
    @Environment(Store.self) private var store
    @AppStorage(RefreshPreference.storageKey) private var preference = RefreshPreference.push

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(L("ui.refresh_settings.mode")).font(.figtree(14, .semibold))
            SegmentedPicker(selection: $preference, options: RefreshPreference.allCases, label: \.label)
                .accessibilityLabel(L("ui.refresh_settings.message_refresh"))
            Muted(preference.detail, size: 12)
            if let view = store.notifications {
                Label(
                    view.realtimeLive ? L("ui.refresh_settings.notification_channel_connected")
                        : L("ui.refresh_settings.notification_channel_disconnected"),
                    systemImage: view.realtimeLive
                        ? "antenna.radiowaves.left.and.right" : "antenna.radiowaves.left.and.right.slash"
                )
                .font(.figtree(12))
                .foregroundStyle(view.realtimeLive ? Theme.muted : Theme.danger)
            }
        }
        .card(padding: 18)
        .onChange(of: preference) { _, value in store.send(.setRefreshMode(mode: value.core)) }
    }
}

/// Settings home row for Actualisation: the mode, and whether push is down.
struct RefreshSettingsLink: View {
    @Environment(Store.self) private var store
    @AppStorage(RefreshPreference.storageKey) private var preference = RefreshPreference.push
    let open: () -> Void
    var body: some View {
        SettingsLink(symbol: "arrow.triangle.2.circlepath", title: L("ui.refresh_settings.message_refresh"),
                     summary: summary, open: open)
    }
    private var summary: String {
        guard let view = store.notifications, !view.realtimeLive else { return preference.label }
        return L("ui.refresh_settings.notification_channel_disconnected_2", ["label": preference.label])
    }
}
