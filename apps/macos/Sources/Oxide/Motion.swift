import AppKit
import SwiftUI

/// The app's few animation curves, all dropped when macOS asks to reduce motion.
enum Motion {
    /// Hover and press feedback: quick enough not to lag the pointer.
    static var hover: Animation? { allowed(.easeOut(duration: 0.12)) }
    /// A selection moving between items (rail, tabs, chips).
    static var select: Animation? { allowed(.spring(response: 0.3, dampingFraction: 0.85)) }
    /// Screens, panels and banners coming and going.
    static var standard: Animation? { allowed(.easeInOut(duration: 0.2)) }
    /// Badges and dots popping in.
    static var pop: Animation? { allowed(.spring(response: 0.25, dampingFraction: 0.6)) }

    static var reduced: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }
    private static func allowed(_ animation: Animation) -> Animation? { reduced ? nil : animation }
}

extension AnyTransition {
    /// Banners and bars sliding down from their edge.
    static var slideDown: AnyTransition { .move(edge: .top).combined(with: .opacity) }
    /// Floating pills rising from the bottom.
    static var rise: AnyTransition { .offset(y: 12).combined(with: .opacity) }
    /// Badges and unread dots.
    static var pop: AnyTransition { .scale(scale: 0.4).combined(with: .opacity) }
}

/// A light band sweeping across placeholder shapes, Teams-style; still when motion is reduced.
private struct Shimmer: ViewModifier {
    @State private var phase: CGFloat = -1
    func body(content: Content) -> some View {
        content
            .overlay {
                GeometryReader { proxy in
                    LinearGradient(
                        colors: [.clear, .white.opacity(0.35), .clear],
                        startPoint: .leading, endPoint: .trailing
                    )
                    .frame(width: proxy.size.width * 0.6)
                    .offset(x: phase * proxy.size.width * 1.6)
                    .blendMode(.plusLighter)
                }
                .allowsHitTesting(false)
            }
            .mask(content)
            .onAppear {
                guard !Motion.reduced else { return }
                withAnimation(.linear(duration: 1.3).repeatForever(autoreverses: false)) { phase = 1 }
            }
    }
}

extension View {
    func shimmer() -> some View { modifier(Shimmer()) }
}

/// Grey bars standing in for list rows while the first page loads.
struct SkeletonList: View {
    var rows = 6
    var avatar: CGFloat = 40
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(0..<rows, id: \.self) { index in
                HStack(spacing: 12) {
                    Circle().frame(width: avatar, height: avatar)
                    VStack(alignment: .leading, spacing: 7) {
                        RoundedRectangle(cornerRadius: 4).frame(width: Self.titleWidth(index), height: 11)
                        RoundedRectangle(cornerRadius: 4).frame(maxWidth: .infinity).frame(height: 9)
                            .padding(.trailing, Self.previewInset(index))
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                // Fades towards the bottom, so the list reads as "more to come" rather than a fixed block.
                .opacity(1 - Double(index) / Double(rows) * 0.6)
            }
        }
        .foregroundStyle(Theme.input)
        .shimmer()
        .accessibilityHidden(true)
    }
    /// Varied but stable widths, so rows do not look stamped.
    private static func titleWidth(_ index: Int) -> CGFloat { [120, 90, 140, 105, 80, 130][index % 6] }
    private static func previewInset(_ index: Int) -> CGFloat { [30, 70, 10, 50, 90, 40][index % 6] }
}

/// A spinner with its label, shown only once loading lasts past a blink so fast loads do not flash.
struct LoadingLabel: View {
    let text: String
    var size: CGFloat = 12
    @State private var shown = false
    var body: some View {
        HStack(spacing: 8) {
            ProgressView().controlSize(.small)
            Muted(text, size: size)
        }
        .opacity(shown ? 1 : 0)
        .task {
            try? await Task.sleep(for: .milliseconds(120))
            withAnimation(Motion.standard) { shown = true }
        }
        .accessibilityElement(children: .combine)
    }
}

/// An arrow that turns while a refresh is running.
struct RefreshButton: View {
    let help: String
    var spinning = false
    var side: CGFloat = 34
    let action: () -> Void
    @State private var angle: Double = 0
    var body: some View {
        Button(action: action) {
            Image(systemName: "arrow.clockwise")
                .font(.system(size: 16))
                .rotationEffect(.degrees(angle))
                .frame(width: side, height: side)
        }
        .buttonStyle(GhostButtonStyle())
        .help(help)
        .accessibilityLabel(help)
        .onAppear { spin(spinning) }
        .onChange(of: spinning) { spin(spinning) }
    }
    private func spin(_ on: Bool) {
        if on && !Motion.reduced {
            withAnimation(.linear(duration: 0.9).repeatForever(autoreverses: false)) { angle = 360 }
        } else {
            // Settles on a whole turn instead of snapping back mid-spin.
            withAnimation(Motion.standard) { angle = 0 }
        }
    }
}
