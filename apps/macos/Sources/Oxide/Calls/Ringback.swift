import AppKit
import OxideCore

/// French ringback tone (440 Hz, 1.5 s on, 3.5 s off) while an outgoing call rings.
@MainActor
final class OutgoingRingback {
    private var ringing: String?
    private var sound: NSSound?

    func sync(_ part: CallPart?, store: Store) {
        let call = part?.active.flatMap { $0.phase == .ringing && !$0.isMeeting && !$0.isTest ? $0 : nil }
        guard call?.id != ringing else { return }
        stop()
        guard let call, store.notifications?.callSounds ?? true else { return }
        ringing = call.id
        let sound = NSSound(data: Self.tone)
        sound?.loops = true
        sound?.volume = 0.35
        sound?.playOnChosenSpeaker()
        self.sound = sound
    }

    private func stop() {
        sound?.stop()
        sound = nil
        ringing = nil
    }

    /// One 5 s period as 16-bit mono WAV.
    private static let tone: Data = {
        let rate = 16_000
        let on = rate * 3 / 2, total = rate * 5, fade = rate / 100
        var samples = [Int16](repeating: 0, count: total)
        for i in 0..<on {
            let envelope = min(1, Double(min(i, on - i)) / Double(fade))
            samples[i] = Int16(sin(2 * .pi * 440 * Double(i) / Double(rate)) * envelope * 12_000)
        }
        var data = Data()
        func append<T: FixedWidthInteger>(_ value: T) { withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) } }
        let bytes = UInt32(total * 2)
        data.append(contentsOf: Array("RIFF".utf8)); append(36 + bytes)
        data.append(contentsOf: Array("WAVEfmt ".utf8)); append(UInt32(16)); append(UInt16(1)); append(UInt16(1))
        append(UInt32(rate)); append(UInt32(rate * 2)); append(UInt16(2)); append(UInt16(16))
        data.append(contentsOf: Array("data".utf8)); append(bytes)
        samples.forEach { append($0) }
        return data
    }()
}
