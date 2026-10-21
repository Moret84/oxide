import AppKit
import SwiftUI
import OxideCore

/// "Indigo net" palette, with dark variants.
enum Theme {
    static let background = dynamic(0xF4F4F9, 0x1B1B22)
    static let surface = dynamic(0xFFFFFF, 0x24242C)
    static let conversation = dynamic(0xF7F7FB, 0x1F1F26)
    static let border = dynamic(0xE4E4EE, 0x34343F)
    static let borderStrong = dynamic(0xD6D6E4, 0x44444F)
    static let bubbleBorder = dynamic(0xE6E6F0, 0x383843)
    static let text = dynamic(0x1E1E2C, 0xEDEDF4)
    static let textSoft = dynamic(0x3F3F52, 0xC9C9D6)
    static let muted = dynamic(0x5C5C70, 0xA4A4B6)
    static let subtle = dynamic(0x6B6B80, 0x8F8FA3)
    static let accent = dynamic(0x5B5FC7, 0x8A8DF0)
    /// The app icon's violet gradient, the same in both modes.
    static let brandTop = rgb(0x7B7FE0)
    static let brandBottom = rgb(0x4B4FB8)
    static let accentSoft = dynamic(0xECECF9, 0x34355C)
    static let accentText = dynamic(0x4B4FB8, 0xB4B6FA)
    static let mineBubble = dynamic(0xE5E7FA, 0x34365E)
    static let input = dynamic(0xF0F0F6, 0x2C2C35)
    static let danger = dynamic(0xC4314B, 0xF06A80)
    static let errorBackground = dynamic(0xFCE8EC, 0x4A2029)
    static let errorText = dynamic(0x8E1F33, 0xFFC2CC)
    static let pdfBackground = dynamic(0xFCE8EC, 0x4A2029)
    static let pdfText = dynamic(0xA3243B, 0xFFB0BD)

    private static let avatars: [Color] = [0x5B5FC7, 0xB4467A, 0x2F7D6D, 0xA35A12, 0x3D6FB6, 0x7A4FB5].map(rgb)
    static func avatar(_ seed: String) -> Color {
        let hash = seed.utf8.reduce(UInt32(0)) { $0 &* 31 &+ UInt32($1) }
        return avatars[Int(hash % UInt32(avatars.count))]
    }

    static func rgb(_ hex: UInt32) -> Color {
        Color(nsColor: nsColor(hex))
    }
    private static func nsColor(_ hex: UInt32) -> NSColor {
        NSColor(
            srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: 1
        )
    }
    private static func dynamic(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? nsColor(dark) : nsColor(light)
        })
    }
}

/// User-chosen appearance, overriding the macOS one unless `.system`.
enum Appearance: String, CaseIterable, Identifiable {
    case system, light, dark

    static let storageKey = "appearance"

    var id: Self { self }
    var label: String {
        switch self {
        case .system: L("ui.theme.system")
        case .light: L("ui.theme.light")
        case .dark: L("ui.theme.dark")
        }
    }

    /// Set at the NSApp level so AppKit views (NSTextView) and dynamic NSColors follow too.
    @MainActor func apply() {
        NSApp.appearance = switch self {
        case .system: nil
        case .light: NSAppearance(named: .aqua)
        case .dark: NSAppearance(named: .darkAqua)
        }
    }
}

extension Font {
    /// Figtree is registered from the app bundle (ATSApplicationFontsPath); system font otherwise.
    static func figtree(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        if figtreeAvailable {
            return .custom("Figtree", size: size).weight(weight)
        }
        return .system(size: size, weight: weight)
    }
    /// Looked up once: this runs for nearly every text drawn.
    private static let figtreeAvailable = NSFont(name: "Figtree-Regular", size: 13) != nil
}

extension View {
    func card(radius: CGFloat = 12, padding: CGFloat = 0) -> some View {
        self.padding(padding)
            .background(Theme.surface, in: RoundedRectangle(cornerRadius: radius))
            .overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(Theme.border))
    }
}

struct PrimaryButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.figtree(13, .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, 16)
            .frame(minHeight: 32)
            .background(
                enabled ? Theme.accent.opacity(configuration.isPressed ? 0.8 : 1) : Theme.accent.opacity(0.4),
                in: RoundedRectangle(cornerRadius: 8)
            )
            .pointerCursor()
    }
}

struct OutlineButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.figtree(13))
            .foregroundStyle(enabled ? Theme.textSoft : Theme.subtle)
            .padding(.horizontal, 12)
            .frame(minHeight: 32)
            .background(configuration.isPressed ? Theme.input : Theme.surface, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.borderStrong))
            .pointerCursor()
    }
}

struct GhostButtonStyle: ButtonStyle {
    var accent = false
    @State private var hovered = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(accent ? Theme.accentText : Theme.muted)
            .background(
                hovered || configuration.isPressed ? Theme.input : .clear,
                in: RoundedRectangle(cornerRadius: 8)
            )
            .contentShape(Rectangle())
            .onHover { if hovered != $0 { hovered = $0 } }
            .pointerCursor()
    }
}

/// Segmented control in the app palette (the AppKit one uses the system accent).
struct SegmentedPicker<Value: Hashable & Identifiable>: View {
    @Binding var selection: Value
    let options: [Value]
    let label: (Value) -> String
    var body: some View {
        HStack(spacing: 2) {
            ForEach(options) { option in
                let active = option == selection
                Button { selection = option } label: {
                    Text(label(option))
                        .font(.figtree(13, active ? .semibold : .regular))
                        .foregroundStyle(active ? Theme.accentText : Theme.textSoft)
                        .padding(.horizontal, 14)
                        .frame(minHeight: 26)
                        .background(active ? Theme.accentSoft : .clear, in: RoundedRectangle(cornerRadius: 6))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .pointerCursor()
                .accessibilityAddTraits(active ? .isSelected : [])
            }
        }
        .padding(3)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.borderStrong))
    }
}

/// Ghost button (34×34 by default) with an SF Symbol and a tooltip.
struct IconButton: View {
    let symbol: String
    let help: String
    var side: CGFloat = 34
    var glyph: CGFloat = 16
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: glyph))
                .frame(width: side, height: side)
        }
        .buttonStyle(GhostButtonStyle())
        .help(help)
        .accessibilityLabel(help)
    }
}

struct Avatar: View {
    @Environment(Store.self) private var store
    let name: String
    let id: String
    var size: CGFloat = 32
    var group = false
    /// Direct chats: the other person's availability, drawn on the bottom-right corner.
    var presence: PresenceView? = nil
    var body: some View {
        let photo = store.avatarBox(id).image
        let shape = RoundedRectangle(cornerRadius: group ? size * 0.25 : size / 2)
        Group {
            if let photo {
                Image(nsImage: photo).resizable().scaledToFill()
            } else {
                Text(initials(name))
            }
        }
        .font(.figtree(size * 0.32, .semibold))
        .foregroundStyle(.white)
        .frame(width: size, height: size)
        .background(Theme.avatar(id), in: shape)
        .clipShape(shape)
        // Asks again if the core evicts the photo while it is on screen.
        .task(id: "\(id)|\(name)|\(photo == nil)") {
            if photo == nil { store.requestAvatar(id: id, name: name) }
        }
        .overlay(alignment: .bottomTrailing) {
            if let presence {
                // Whole, even points: a fractional badge blurs and shifts its mark.
                PresenceBadge(presence: presence, size: max(10, (size * 0.16).rounded() * 2))
                    .offset(x: (size * 0.04).rounded(), y: (size * 0.04).rounded())
            }
        }
        .accessibilityHidden(true)
    }
    private func initials(_ name: String) -> String {
        let words = name.split(whereSeparator: \.isWhitespace)
        let value: String
        if words.count > 1, let first = words.first?.first, let last = words.last?.first {
            value = "\(first)\(last)"
        } else {
            value = String(name.prefix(2))
        }
        return value.uppercased()
    }
}

/// The web client's presence badge, ringed in the background colour; out of office turns it into a left arrow.
struct PresenceBadge: View {
    let presence: PresenceView
    let size: CGFloat
    private static let available = Theme.rgb(0x6BB700)
    private static let busy = Theme.rgb(0xD13438)
    private static let away = Theme.rgb(0xF8A800)
    private static let offline = Theme.rgb(0x8A8886)
    private static let outOfOffice = Theme.rgb(0xB4009E)
    var body: some View {
        ZStack {
            if hollow {
                Circle().strokeBorder(color, lineWidth: lineWidth)
            } else {
                Circle().fill(color)
            }
            // Drawn strokes: SF Symbols sit on a text baseline and lose their centre at badge sizes.
            mark
                .stroke(hollow ? color : .white,
                        style: StrokeStyle(lineWidth: lineWidth, lineCap: .round, lineJoin: .round))
        }
        .frame(width: size, height: size)
        .background(Circle().fill(Theme.surface).padding(-1.5))
        .help(PresenceBadge.label(presence))
        .accessibilityLabel(PresenceBadge.label(presence))
    }
    private var lineWidth: CGFloat { max(1.2, size * 0.13) }
    /// Offline and unknown are a ring; the others a filled disc.
    private var hollow: Bool {
        !presence.outOfOffice && (presence.status == .offline || presence.status == .unknown)
    }
    /// The mark inside the circle, in a unit square centred on the badge.
    private var mark: BadgeMark {
        if presence.outOfOffice {
            return BadgeMark([[(0.70, 0.5), (0.30, 0.5)], [(0.47, 0.33), (0.30, 0.5), (0.47, 0.67)]])
        }
        switch presence.status {
        case .available: return BadgeMark([[(0.29, 0.52), (0.44, 0.66), (0.71, 0.37)]])
        case .busy, .unknown: return BadgeMark([])
        case .doNotDisturb: return BadgeMark([[(0.30, 0.5), (0.70, 0.5)]])
        case .beRightBack, .away: return BadgeMark([[(0.5, 0.28), (0.5, 0.52), (0.67, 0.62)]])
        case .offline: return BadgeMark([[(0.36, 0.36), (0.64, 0.64)], [(0.64, 0.36), (0.36, 0.64)]])
        }
    }
    private var color: Color {
        switch presence.status {
        case .available: return Self.available
        case .busy, .doNotDisturb: return Self.busy
        case .beRightBack, .away:
            return presence.outOfOffice ? Self.outOfOffice : Self.away
        case .offline, .unknown:
            return presence.outOfOffice ? Self.outOfOffice : Self.offline
        }
    }
    static func label(_ presence: PresenceView) -> String {
        let status = switch presence.status {
        case .available: L("ui.presence.available")
        case .busy: L("ui.presence.busy")
        case .doNotDisturb: L("ui.presence.do_not_disturb")
        case .beRightBack: L("ui.presence.be_right_back")
        case .away: L("ui.presence.away")
        case .offline: L("ui.presence.offline")
        case .unknown: L("ui.presence.unknown")
        }
        return presence.outOfOffice ? L("ui.presence.out_of_office", ["status": status]) : status
    }
}

/// Polylines in a unit square, scaled to the badge.
struct BadgeMark: Shape {
    let lines: [[(CGFloat, CGFloat)]]
    init(_ lines: [[(CGFloat, CGFloat)]]) { self.lines = lines }
    func path(in rect: CGRect) -> Path {
        var path = Path()
        for line in lines {
            path.addLines(line.map { CGPoint(x: rect.minX + $0.0 * rect.width, y: rect.minY + $0.1 * rect.height) })
        }
        return path
    }
}

struct Muted: View {
    let text: String
    let size: CGFloat
    init(_ text: String, size: CGFloat = 12) {
        self.text = text
        self.size = size
    }
    var body: some View {
        Text(text).font(.figtree(size)).foregroundStyle(Theme.muted)
    }
}

/// A screen's load error across its width, with a button to try again.
struct ErrorBanner: View {
    let text: String
    let retry: String
    var disabled = false
    let action: () -> Void
    var body: some View {
        HStack {
            Image(systemName: "exclamationmark.triangle")
            Text(text).textSelection(.enabled)
            Spacer()
            Button(retry, action: action).disabled(disabled)
        }
        .font(.figtree(12)).foregroundStyle(Theme.errorText).padding(12).background(Theme.errorBackground)
    }
}
