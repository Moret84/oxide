import AppKit
import OxideCore
import WebKit

/// Microsoft sign-in, stopped on the return URL for the core's PKCE exchange; its own cookie store allows later `prompt=none` silent sign-ins.
@MainActor
final class LoginWindow: NSObject, NSWindowDelegate, WKNavigationDelegate {
    var onReturn: (String) -> Void = { _ in }
    var onCancel: () -> Void = {}
    var onSilentFailure: () -> Void = {}
    private var window: NSWindow?
    private var returned = false
    private var silentView: WKWebView?
    private var silentTimeout: Task<Void, Never>?

    /// Only for the Microsoft sign-in; nothing else in the app loads pages with it.
    private static let store = WKWebsiteDataStore(forIdentifier: UUID(uuidString: "6D2A3C1E-4B7F-4E0A-9C5D-2F1B8A7E6D40")!)
    /// A silent sign-in is a few redirects; past this, Microsoft is waiting for a person.
    private static let silentLimit: Duration = .seconds(30)

    private func configuration() -> WKWebViewConfiguration {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = Self.store
        return configuration
    }

    func open(_ url: String) {
        guard let target = URL(string: url) else { return onCancel() }
        close()
        returned = false
        let web = WKWebView(frame: .zero, configuration: configuration())
        web.navigationDelegate = self
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 540, height: 740),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered, defer: false
        )
        window.title = L("ui.login_window.microsoft_sign_in_oxide")
        window.contentView = web
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window
        web.load(URLRequest(url: target))
    }

    /// Closes without reporting a cancellation (the core asked for it).
    func close() {
        guard let window else { return }
        returned = true
        window.close()
    }

    /// Loads a `prompt=none` URL off screen: the return goes to `onReturn`, anything else to `onSilentFailure`.
    func silent(_ url: String) {
        endSilent()
        guard let target = URL(string: url) else { return onSilentFailure() }
        let web = WKWebView(frame: .zero, configuration: configuration())
        web.navigationDelegate = self
        silentView = web
        silentTimeout = Task { [weak self] in
            try? await Task.sleep(for: Self.silentLimit)
            guard !Task.isCancelled, let self, self.silentView === web else { return }
            self.failSilent()
        }
        web.load(URLRequest(url: target))
    }

    /// Signing out: no later silent sign-in may reuse this account.
    func forget() {
        endSilent()
        Self.store.removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast) {}
    }

    private func endSilent() {
        silentTimeout?.cancel()
        silentTimeout = nil
        silentView?.stopLoading()
        silentView?.navigationDelegate = nil
        silentView = nil
    }

    private func failSilent() {
        endSilent()
        onSilentFailure()
    }

    func windowWillClose(_ notification: Notification) {
        if let web = window?.contentView as? WKWebView {
            web.stopLoading()
            web.navigationDelegate = nil
        }
        window = nil
        if !returned {
            onCancel()
        }
    }

    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        let url = navigationAction.request.url?.absoluteString ?? ""
        let silent = webView === silentView
        if isLoginCallback(url: url) {
            decisionHandler(.cancel)
            if silent { endSilent() } else { returned = true }
            onReturn(url)
            return
        }
        let allowed = isLoginAllowed(url: url)
        decisionHandler(allowed ? .allow : .cancel)
        if silent && !allowed { failSilent() }
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        if webView === silentView { failSilent() }
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        if webView === silentView { failSilent() }
    }
}
