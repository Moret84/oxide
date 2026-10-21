import AppKit
import SwiftUI
import OxideCore

/// Swift-side language and texts (the core's crates/teams-i18n catalog), set by `LanguagePreference.apply()`.
enum L10n {
    nonisolated(unsafe) static var english = false
    /// Loaded on first use (French until a language is applied), reloaded by `apply()`.
    nonisolated(unsafe) static var table: [String: String] = translations()
    static var locale: Locale { Locale(identifier: english ? "en_US" : "fr_FR") }
}

/// The catalog text of `key`, its `{name}` placeholders filled from `args`.
func L(_ key: String, _ args: [String: Any] = [:]) -> String {
    var text = L10n.table[key] ?? key
    for (name, value) in args {
        text = text.replacingOccurrences(of: "{\(name)}", with: "\(value)")
    }
    return text
}

/// `key.one` or `key.other` for `n`, with `{n}` filled; French takes the singular for 0 too.
func Lp(_ key: String, _ n: Int, _ args: [String: Any] = [:]) -> String {
    let one = L10n.english ? n == 1 : n <= 1
    return L("\(key).\(one ? "one" : "other")", args.merging(["n": n]) { value, _ in value })
}

/// The app's language: the system's unless chosen in Settings, kept in the user defaults and handed to the core.
enum LanguagePreference: String, CaseIterable, Identifiable {
    case system, french, english

    static let storageKey = "language"

    static var stored: Self {
        UserDefaults.standard.string(forKey: storageKey).flatMap(Self.init(rawValue:)) ?? .system
    }

    var id: Self { self }
    var label: String {
        switch self {
        case .system: L("ui.localization.system")
        case .french: "Français"
        case .english: "English"
        }
    }
    var resolved: Language {
        switch self {
        case .french: .french
        case .english: .english
        case .system: Self.systemLanguage
        }
    }
    /// Whichever of French and English ranks first globally; the app's own `AppleLanguages` holds the override.
    static var systemLanguage: Language {
        let global = UserDefaults.standard.persistentDomain(forName: UserDefaults.globalDomain)
        let ranked = global?["AppleLanguages"] as? [String] ?? Locale.preferredLanguages
        for code in ranked {
            if code.hasPrefix("fr") { return .french }
            if code.hasPrefix("en") { return .english }
        }
        return .english
    }

    /// Sets the Swift and core language; AppKit's menus follow `AppleLanguages`, read at launch only.
    func apply() {
        let language = resolved
        L10n.english = language == .english
        setLanguage(language: language)
        L10n.table = translations()
        switch self {
        case .system: UserDefaults.standard.removeObject(forKey: "AppleLanguages")
        case .french: UserDefaults.standard.set(["fr"], forKey: "AppleLanguages")
        case .english: UserDefaults.standard.set(["en"], forKey: "AppleLanguages")
        }
    }
}

/// Settings › Langue.
struct LanguageSettingsSection: View {
    @Environment(Store.self) private var store
    @AppStorage(LanguagePreference.storageKey) private var preference = LanguagePreference.system

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(L("ui.localization.app_language")).font(.figtree(14, .semibold))
            SegmentedPicker(selection: selection, options: LanguagePreference.allCases, label: \.label)
                .accessibilityLabel(L("ui.localization.language"))
            Muted(L("ui.localization.system_menus_follow_next_launch"), size: 12)
        }
        .card(padding: 18)
    }

    /// Applies the language before the stored value changes, so redrawn views already read it.
    private var selection: Binding<LanguagePreference> {
        Binding(
            get: { preference },
            set: { value in
                value.apply()
                store.setLanguage(value.resolved)
                preference = value
                // The rebuilt root sends Settings back to its home: return to this page.
                DispatchQueue.main.async { store.settingsPage = .language }
            }
        )
    }
}
