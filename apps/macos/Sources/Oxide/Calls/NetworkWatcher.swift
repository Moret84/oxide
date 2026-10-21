import AppKit
import Network

/// Reports wake-ups and network path changes, debounced, so push and call media reconnect without waiting for timeouts.
@MainActor
final class NetworkWatcher {
    var onChange: (() -> Void)?
    /// Whether a usable path exists, on every update (the offline banner).
    var onStatus: ((Bool) -> Void)?
    private let monitor = NWPathMonitor()
    private var lastPath: String?
    private var pending: Task<Void, Never>?
    private var wake: NSObjectProtocol?

    func start() {
        monitor.pathUpdateHandler = { [weak self] path in
            let signature = Self.signature(path)
            Task { @MainActor in self?.path(signature) }
        }
        monitor.start(queue: DispatchQueue(label: "network-watcher"))
        wake = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didWakeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.changed() }
        }
    }

    /// Interfaces in use and whether the path is usable; the first value is the baseline.
    nonisolated static func signature(_ path: NWPath) -> String {
        let interfaces = path.availableInterfaces.map(\.name).joined(separator: ",")
        return "\(path.status == .satisfied)|\(interfaces)"
    }

    private func path(_ signature: String) {
        defer { lastPath = signature }
        onStatus?(signature.hasPrefix("true"))
        guard let lastPath, lastPath != signature, signature.hasPrefix("true") else { return }
        changed()
    }

    private func changed() {
        pending?.cancel()
        pending = Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            self?.onChange?()
        }
    }
}
