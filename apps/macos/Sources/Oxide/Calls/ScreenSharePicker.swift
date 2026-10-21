import AppKit
import LiveKitWebRTC
import SwiftUI

/// Drops the strong `self` libwebrtc's desktop capturer/list native half keeps (else never freed, ~60 MB); once only (webrtc-xcframework 144.7559.15).
func releaseNativeSelfReference(_ object: AnyObject) {
    Unmanaged.passUnretained(object).release()
}

/// A picked source with its lists, which must live until the capture starts since it reads their native memory.
struct ScreenChoice {
    let source: LKRTCDesktopSource
    let lists: DesktopSources
}

/// Screens and windows that can be shared, with their thumbnails (libwebrtc's desktop lists).
@MainActor
@Observable
final class DesktopSources: NSObject {
    private(set) var screens: [LKRTCDesktopSource] = []
    private(set) var windows: [LKRTCDesktopSource] = []
    /// Bumped when libwebrtc reports a new thumbnail, so the grid redraws.
    private(set) var revision = 0
    @ObservationIgnored private var lists: [LKRTCDesktopMediaList] = []

    /// libwebrtc can leave a source without name or thumbnail, though its header says otherwise.
    static func name(of source: LKRTCDesktopSource) -> String {
        let name: String? = source.name
        return name ?? ""
    }

    static func thumbnail(of source: LKRTCDesktopSource) -> NSImage? {
        let thumbnail: NSImage? = source.thumbnail
        return thumbnail
    }

    func load() {
        if lists.isEmpty {
            lists = [LKRTCDesktopMediaList(type: .screen, delegate: self), LKRTCDesktopMediaList(type: .window, delegate: self)]
        }
        for list in lists { _ = list.updateSourceList(true, updateAllThumbnails: true) }
        refresh()
    }

    /// Frees the lists and their streams; sources keep name and thumbnail but can no longer start a capture.
    func close() {
        screens = []
        windows = []
        lists.forEach(releaseNativeSelfReference)
        lists = []
    }

    private func refresh() {
        guard lists.count == 2 else { return }
        screens = lists[0].getSources()
        let own = Bundle.main.localizedInfoDictionary?["CFBundleName"] as? String ?? ProcessInfo.processInfo.processName
        windows = lists[1].getSources().filter { source in
            let name = Self.name(of: source)
            return !name.trimmingCharacters(in: .whitespaces).isEmpty && name != own
        }
    }
}

extension DesktopSources: LKRTCDesktopMediaListDelegate {
    nonisolated func didDesktopSourceAdded(_ source: LKRTCDesktopSource) {}
    nonisolated func didDesktopSourceRemoved(_ source: LKRTCDesktopSource) {}
    nonisolated func didDesktopSourceNameChanged(_ source: LKRTCDesktopSource) {}
    nonisolated func didDesktopSourceThumbnailChanged(_ source: LKRTCDesktopSource) {
        Task { @MainActor in
            self.revision += 1
            self.refresh()
        }
    }
}

/// Share sheet: pick a screen or a window, then share it.
struct ScreenSharePicker: View {
    /// Takes the lists along: whoever receives the choice closes them.
    let share: (ScreenChoice) -> Void
    let cancel: () -> Void
    @State private var sources = DesktopSources()
    @State private var shared = false
    @State private var selected: String?
    @State private var allowed = CGPreflightScreenCaptureAccess()

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(L("ui.screen_share_picker.share_screen")).font(.figtree(18, .semibold))
            if allowed {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        section(L("ui.screen_share_picker.screens"), sources.screens)
                        if !sources.windows.isEmpty { section(L("ui.screen_share_picker.windows"), sources.windows) }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            } else {
                Muted(L("ui.screen_share_picker.oxide_must_be_allowed_in_system"), size: 13)
                Button(L("ui.screen_share_picker.open_settings")) {
                    _ = CGRequestScreenCaptureAccess()
                    NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")!)
                }
                .buttonStyle(OutlineButtonStyle())
                Spacer()
            }
            HStack {
                Spacer()
                Button(L("ui.screen_share_picker.cancel"), action: cancel).buttonStyle(OutlineButtonStyle()).keyboardShortcut(.cancelAction)
                Button(L("ui.screen_share_picker.share")) {
                    guard let chosen else { return }
                    shared = true
                    share(ScreenChoice(source: chosen, lists: sources))
                }
                .buttonStyle(PrimaryButtonStyle())
                .keyboardShortcut(.defaultAction)
                .disabled(!allowed || chosen == nil)
            }
        }
        .padding(22)
        .frame(width: 640, height: 520)
        .onAppear {
            if !allowed { allowed = CGRequestScreenCaptureAccess() }
            guard allowed else { return }
            sources.load()
            selected = sources.screens.first?.sourceId
        }
        .onDisappear {
            if !shared { sources.close() }
        }
    }

    private var chosen: LKRTCDesktopSource? {
        (sources.screens + sources.windows).first { $0.sourceId == selected }
    }

    private func section(_ title: String, _ items: [LKRTCDesktopSource]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.figtree(13, .semibold)).foregroundStyle(Theme.muted)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 170), spacing: 12)], spacing: 12) {
                ForEach(items, id: \.sourceId) { source in
                    let _ = sources.revision
                    let isSelected = selected == source.sourceId
                    Button { selected = source.sourceId } label: {
                        VStack(spacing: 6) {
                            Group {
                                if let thumbnail = DesktopSources.thumbnail(of: source) {
                                    Image(nsImage: thumbnail).resizable().aspectRatio(contentMode: .fit)
                                } else {
                                    Image(systemName: source.sourceType == .window ? "macwindow" : "display")
                                        .font(.system(size: 28)).foregroundStyle(.white.opacity(0.7))
                                }
                            }
                            .frame(height: 96)
                            .frame(maxWidth: .infinity)
                            .background(Color.black.opacity(0.85), in: RoundedRectangle(cornerRadius: 8))
                            Text(DesktopSources.name(of: source)).font(.figtree(12)).lineLimit(1).truncationMode(.middle)
                        }
                        .padding(6)
                        .background(isSelected ? Theme.accentSoft : Color.clear, in: RoundedRectangle(cornerRadius: 10))
                        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(isSelected ? Theme.accent : Theme.border, lineWidth: isSelected ? 2 : 1))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .pointerCursor()
                    .accessibilityAddTraits(isSelected ? .isSelected : [])
                }
            }
        }
    }
}
