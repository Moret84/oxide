import SwiftUI
import OxideCore

/// "Alice est en train d’écrire" under the messages, with Teams' three bouncing dots.
struct TypingIndicator: View {
    let text: String?

    var body: some View {
        HStack(spacing: 8) {
            if let text {
                TypingDots()
                Text(text).font(.figtree(12)).foregroundStyle(Theme.muted).lineLimit(1)
                    .transition(.opacity.combined(with: .offset(y: 4)))
            }
        }
        .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
        .padding(.horizontal, 24)
        .padding(.top, 2)
        .animation(.easeOut(duration: 0.15), value: text)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(text ?? "")
    }
}

/// Three dots rising one after the other, in a loop.
/// The bounce is an endless custom animation, so the render loop moves the dots without re-running any body.
struct TypingDots: View {
    var body: some View {
        HStack(spacing: 3) {
            ForEach(0..<3, id: \.self) { TypingDot(index: $0) }
        }
        .frame(height: 10)
    }

    /// Each dot bounces for a third of a 1.2 s cycle, the next one starting 0.15 s later.
    static func offset(time: TimeInterval, index: Int) -> CGFloat {
        let phase = (time - Double(index) * 0.15).truncatingRemainder(dividingBy: 1.2)
        let normalized = phase < 0 ? phase + 1.2 : phase
        guard normalized < 0.4 else { return 0 }
        return -3 * CGFloat(sin(normalized / 0.4 * .pi))
    }
}

private struct TypingDot: View {
    let index: Int
    @State private var up = false
    var body: some View {
        Circle()
            .fill(Theme.muted)
            .frame(width: 5, height: 5)
            .offset(y: up ? -3 : 0)
            .animation(Animation(TypingBounce(index: index)), value: up)
            .onAppear { up = true }
    }
}

/// Follows `TypingDots.offset` forever: the full 3 pt rise is scaled by the dot's bounce at each instant.
private struct TypingBounce: CustomAnimation {
    let index: Int
    func animate<V: VectorArithmetic>(value: V, time: TimeInterval, context: inout AnimationContext<V>) -> V? {
        value.scaled(by: Double(TypingDots.offset(time: time, index: index) / -3))
    }
}

/// Teams' status of my latest messages: a check for sent, an eye once read.
struct ReceiptIcon: View {
    let receipt: ReceiptView

    var body: some View {
        Image(systemName: ReceiptIcon.symbol(receipt))
            .font(.system(size: 11))
            .foregroundStyle(receipt == .sent ? Theme.subtle : Theme.accent)
            .help(ReceiptIcon.label(receipt))
            .accessibilityLabel(ReceiptIcon.label(receipt))
    }

    static func symbol(_ receipt: ReceiptView) -> String {
        switch receipt {
        case .sent: "checkmark.circle"
        case .seen, .seenByEveryone: "eye"
        }
    }
    static func label(_ receipt: ReceiptView) -> String {
        switch receipt {
        case .sent: L("ui.typing_indicator.sent")
        case .seen: L("ui.typing_indicator.seen")
        case .seenByEveryone: L("ui.typing_indicator.seen_by_everyone")
        }
    }
}
