import AppKit
import AVFoundation
import CoreAudio

/// Default call devices chosen in Settings; empty or disconnected means the system default.
enum MediaDevicePreferences {
    static let microphoneKey = "call.microphoneUID"
    static let speakerKey = "call.speakerUID"
    static let cameraKey = "call.cameraUniqueID"

    /// The preferred id when it is available, otherwise `nil` (system default).
    static func resolve(_ preferred: String?, available: [String]) -> String? {
        guard let preferred, !preferred.isEmpty, available.contains(preferred) else { return nil }
        return preferred
    }

    private static func stored(_ key: String) -> String? {
        UserDefaults.standard.string(forKey: key)
    }

    static func resolvedMicrophone() -> AudioDeviceID? {
        let available = CoreAudioDevices.devices(input: true)
        guard let uid = resolve(stored(microphoneKey), available: available.map(\.uid)) else {
            return CoreAudioDevices.defaultDevice(input: true)
        }
        return available.first { $0.uid == uid }?.id
    }

    /// UID of the chosen output, or `nil` to play on the system default.
    static func resolvedSpeakerUID() -> String? {
        resolve(stored(speakerKey), available: CoreAudioDevices.devices(input: false).map(\.uid))
    }

    static func resolvedSpeaker() -> AudioDeviceID? {
        resolvedSpeakerUID().flatMap(CoreAudioDevices.device(uid:)) ?? CoreAudioDevices.defaultDevice(input: false)
    }

    static func resolvedCamera() -> AVCaptureDevice? {
        let cameras = Cameras.all()
        if let id = resolve(stored(cameraKey), available: cameras.map(\.uniqueID)) {
            return cameras.first { $0.uniqueID == id }
        }
        return AVCaptureDevice.default(for: .video) ?? cameras.first
    }
}

extension NSSound {
    /// Plays on the output chosen in Settings, like the calls, rather than the system default.
    func playOnChosenSpeaker() {
        if let speaker = MediaDevicePreferences.resolvedSpeakerUID() { playbackDeviceIdentifier = speaker }
        play()
    }
}

/// Microphone and camera access, asked once and explained when refused.
enum MediaPermissions {
    static func request(_ type: AVMediaType) async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: type) {
        case .authorized: return true
        case .notDetermined: return await AVCaptureDevice.requestAccess(for: type)
        default: return false
        }
    }

    static func settingsURL(_ type: AVMediaType) -> URL {
        let pane = type == .audio ? "Privacy_Microphone" : "Privacy_Camera"
        return URL(string: "x-apple.systempreferences:com.apple.preference.security?\(pane)")!
    }
}
