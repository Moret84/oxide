import AVFoundation
import Observation

/// Plays one voicemail at a time on the default sound output chosen in Settings.
@MainActor
@Observable
final class VoicemailPlayer: NSObject, AVAudioPlayerDelegate {
    private(set) var playing: String?
    private(set) var error: String?
    @ObservationIgnored private var player: AVAudioPlayer?

    func play(id: String, data: Data) {
        stop()
        do {
            let player = try AVAudioPlayer(data: data)
            player.delegate = self
            if let uid = MediaDevicePreferences.resolvedSpeakerUID() {
                player.currentDevice = uid
            }
            player.play()
            self.player = player
            playing = id
            error = nil
        } catch {
            self.error = L("ui.voicemail_player.couldn_t_play_the_voicemail")
        }
    }

    func stop() {
        player?.stop()
        player = nil
        playing = nil
    }

    nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        Task { @MainActor in
            if self.player === player { self.stop() }
        }
    }
}
