import AppKit
import OxideCore

/// Rings an incoming call: sound, Dock attention, and an Accept / Decline notification when the app is in the background.
@MainActor
final class IncomingCallAlert {
    private var ringing: String?
    private var sound: NSSound?
    private var attention: Int?

    func sync(_ part: CallPart?, store: Store) {
        let call = part?.active.flatMap { $0.phase == .incoming ? $0 : nil }
        guard call?.id != ringing else { return }
        stop()
        guard let call else { return }
        ringing = call.id
        start(call, callSounds: store.notifications?.callSounds ?? true)
    }

    private func start(_ call: ActiveCallView, callSounds: Bool) {
        if callSounds {
            let sound = NSSound(named: NSSound.Name("Submarine")) ?? NSSound(named: NSSound.Name("Glass"))
            sound?.loops = true
            sound?.playOnChosenSpeaker()
            self.sound = sound
        }
        attention = NSApp.requestUserAttention(.criticalRequest)
        if !NSApp.isActive {
            NotificationRouter.shared.showCall(id: call.id, title: call.title, video: call.offersVideo)
        }
    }

    private func stop() {
        sound?.stop()
        sound = nil
        if let attention { NSApp.cancelUserAttentionRequest(attention) }
        attention = nil
        if let id = ringing { NotificationRouter.shared.removeCall(id: id) }
        ringing = nil
    }
}
