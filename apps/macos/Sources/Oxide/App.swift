import AppKit
import SwiftUI
import OxideCore

@main
struct OxideApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @State private var store = Store(
        demo: demoAvailable()
            && (CommandLine.arguments.contains("--demo")
                || ProcessInfo.processInfo.environment["TEAMS_DEMO"] == "1")
    )
    @AppStorage(Appearance.storageKey) private var appearance = Appearance.system
    /// Read so the menus are rebuilt in a new language.
    @AppStorage(LanguagePreference.storageKey) private var language = LanguagePreference.system

    var body: some Scene {
        Window("Oxide", id: "main") {
            RootView()
                .environment(store)
                .frame(minWidth: 960, minHeight: 600)
                .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
                    store.setActive(true)
                    // Notifications may have been allowed or refused in System Settings meanwhile.
                    NotificationRouter.shared.refreshPermission()
                }
                .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in
                    store.setActive(false)
                }
        }
        .defaultSize(width: 1280, height: 800)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button(L("ui.app.new_chat")) { store.send(.newChat) }
                    .keyboardShortcut("n")
                    .disabled(!store.signedIn)
            }
            CommandGroup(after: .sidebar) {
                Button(L("ui.app.refresh")) { store.send(.refresh) }
                    .keyboardShortcut("r")
                    .disabled(!store.signedIn)
                Picker(L("ui.app.appearance"), selection: $appearance) {
                    ForEach(Appearance.allCases) { Text($0.label).tag($0) }
                }
            }
            CommandMenu(L("ui.app.go_to")) {
                Group {
                    ForEach(Array(RailCommands.items.enumerated()), id: \.offset) { index, item in
                        Button(item.name) { store.send(.navigate(nav: item.nav)) }
                            .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
                    }
                    Divider()
                    Button(L("ui.app.previous_chat")) { store.selectAdjacentChat(-1) }
                        .keyboardShortcut(.upArrow, modifiers: .option)
                    Button(L("ui.app.next_chat")) { store.selectAdjacentChat(1) }
                        .keyboardShortcut(.downArrow, modifiers: .option)
                }
                .disabled(!store.signedIn)
            }
        }
    }
}

/// The rail's destinations, in its order: ⌘1 to ⌘6.
enum RailCommands {
    static var items: [(nav: Nav, symbol: String, name: String)] {
        [
            (.activity, "bell", L("ui.app.activity")),
            (.chat, "bubble.left", L("ui.app.chat")),
            (.teams, "person.3", L("ui.app.teams")),
            (.calendar, "calendar", L("ui.app.calendar")),
            (.calls, "phone", L("ui.app.calls")),
            (.files, "doc", L("ui.app.files")),
        ]
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Launched as a bare executable (swift run): still behave as a regular app.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
        // Before any banner response is delivered, including the one that launched the app.
        MainActor.assumeIsolated { NotificationRouter.shared.configure() }
        Task { @MainActor in await Updater.shared.check(interactive: false) }
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
    /// Quitting mid-call hangs up first so others don't see a frozen participant; gives up after 3 s.
    @MainActor
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let store = Store.current, store.inCall else {
            return .terminateNow
        }
        store.send(.call(action: .hangup))
        Task { @MainActor in
            let deadline = Date().addingTimeInterval(3)
            while Date() < deadline, store.inCall {
                try? await Task.sleep(for: .milliseconds(100))
            }
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}

struct RootView: View {
    @Environment(Store.self) private var store
    @AppStorage(Appearance.storageKey) private var appearance = Appearance.system
    @AppStorage(LanguagePreference.storageKey) private var language = LanguagePreference.system
    var body: some View {
        Group {
            if let shell = store.shell {
                if shell.signedIn {
                    MainView(shell: shell)
                } else {
                    SignInView(shell: shell)
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .transition(.opacity)
        .animation(Motion.standard, value: store.shell == nil)
        .animation(Motion.standard, value: store.shell?.signedIn)
        // A new language rebuilds every view, so none keeps a text of the previous one.
        .id(language)
        .environment(\.locale, L10n.locale)
        .background(Theme.background)
        .font(.figtree(13))
        .foregroundStyle(Theme.text)
        .onAppear { appearance.apply() }
        .onChange(of: appearance) { appearance.apply() }
        .sheet(item: Binding(get: { Updater.shared.prompt }, set: { Updater.shared.prompt = $0 })) {
            UpdatePromptView(release: $0)
        }
    }
}
