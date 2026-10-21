import SwiftUI

/// The app icon's motif (two people in a chat bubble), drawn from AppIcon.svg's coordinates.
struct OxideMark: View {
    /// One colour for a flat bubble, two for a top-to-bottom gradient.
    var bubble: [Color] = [Theme.brandTop, Theme.brandBottom]
    var figure: Color = .white
    /// False draws the bubble alone, for a watermark.
    var people = true

    // The bubble's bounding box in AppIcon.svg.
    private static let box = CGRect(x: 190, y: 236, width: 644, height: 600)

    var body: some View {
        Canvas { context, size in
            let box = Self.box
            let scale = min(size.width / box.width, size.height / box.height)
            context.translateBy(
                x: (size.width - box.width * scale) / 2 - box.minX * scale,
                y: (size.height - box.height * scale) / 2 - box.minY * scale
            )
            context.scaleBy(x: scale, y: scale)
            let shading: GraphicsContext.Shading = bubble.count > 1
                ? .linearGradient(Gradient(colors: bubble), startPoint: CGPoint(x: 0, y: box.minY), endPoint: CGPoint(x: 0, y: box.maxY))
                : .color(bubble.first ?? .white)
            // Two fills: tail and body overlap with opposite windings, which would cut a hole.
            context.fill(Self.bubbleBody, with: shading)
            context.fill(Self.bubbleTail, with: shading)
            guard people else { return }
            context.fill(Self.backPerson, with: .color(figure.opacity(0.72)))
            // The bubble-coloured outline separates the front person from the one behind.
            context.stroke(Self.frontPerson, with: shading, lineWidth: 20)
            context.fill(Self.frontPerson, with: .color(figure))
        }
        .accessibilityHidden(true)
    }

    private static let bubbleBody = Path(roundedRect: CGRect(x: 190, y: 236, width: 644, height: 464), cornerRadius: 132)
    private static let bubbleTail: Path = {
        var path = Path()
        path.move(to: CGPoint(x: 300, y: 640))
        path.addLine(to: CGPoint(x: 300, y: 700))
        path.addCurve(to: CGPoint(x: 236, y: 836), control1: CGPoint(x: 300, y: 764), control2: CGPoint(x: 276, y: 806))
        path.addCurve(to: CGPoint(x: 452, y: 700), control1: CGPoint(x: 326, y: 834), control2: CGPoint(x: 406, y: 794))
        path.addLine(to: CGPoint(x: 452, y: 640))
        path.closeSubpath()
        return path
    }()

    private static let backPerson = person(head: CGPoint(x: 428, y: 380), radius: 60, left: 298, right: 558, top: 470, bottom: 604)
    private static let frontPerson = person(head: CGPoint(x: 592, y: 408), radius: 66, left: 448, right: 736, top: 504, bottom: 644)

    /// A round head above shoulders that curve up from a flat base.
    private static func person(head: CGPoint, radius: CGFloat, left: CGFloat, right: CGFloat, top: CGFloat, bottom: CGFloat) -> Path {
        var path = Path(ellipseIn: CGRect(x: head.x - radius, y: head.y - radius, width: radius * 2, height: radius * 2))
        let middle = (left + right) / 2
        let rise = (bottom - top) * 0.61
        path.move(to: CGPoint(x: left, y: bottom))
        path.addCurve(to: CGPoint(x: middle, y: top), control1: CGPoint(x: left, y: bottom - rise), control2: CGPoint(x: left + (middle - left) * 0.45, y: top))
        path.addCurve(to: CGPoint(x: right, y: bottom), control1: CGPoint(x: right - (right - middle) * 0.45, y: top), control2: CGPoint(x: right, y: bottom - rise))
        path.closeSubpath()
        return path
    }
}
