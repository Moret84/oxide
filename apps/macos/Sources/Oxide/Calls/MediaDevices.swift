import AVFoundation
import AudioToolbox
import CoreAudio
import Observation

struct AudioDevice: Identifiable, Hashable {
    let id: AudioDeviceID
    /// Stable across reboots and reconnections: this is what preferences store.
    let uid: String
    let name: String
}

struct CameraDevice: Identifiable, Hashable {
    var id: String { uniqueID }
    let uniqueID: String
    let name: String
}

/// Thin wrappers over the CoreAudio HAL property API.
enum CoreAudioDevices {
    private static func address(_ selector: AudioObjectPropertySelector,
                                _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
    }

    static func all() -> [AudioDeviceID] {
        var address = address(kAudioHardwarePropertyDevices)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size) == noErr
        else { return [] }
        var ids = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &ids) == noErr
        else { return [] }
        return ids
    }

    static func hasStreams(_ id: AudioDeviceID, input: Bool) -> Bool {
        var address = address(kAudioDevicePropertyStreams,
                              input ? kAudioObjectPropertyScopeInput : kAudioObjectPropertyScopeOutput)
        var size: UInt32 = 0
        return AudioObjectGetPropertyDataSize(id, &address, 0, nil, &size) == noErr && size > 0
    }

    private static func string(_ id: AudioObjectID, _ selector: AudioObjectPropertySelector) -> String? {
        var address = address(selector)
        var value: Unmanaged<CFString>?
        var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        guard AudioObjectGetPropertyData(id, &address, 0, nil, &size, &value) == noErr,
              let value else { return nil }
        return value.takeRetainedValue() as String
    }

    static func uid(_ id: AudioDeviceID) -> String? { string(id, kAudioDevicePropertyDeviceUID) }
    static func name(_ id: AudioDeviceID) -> String? { string(id, kAudioObjectPropertyName) }

    static func defaultDevice(input: Bool) -> AudioDeviceID? {
        var address = address(input ? kAudioHardwarePropertyDefaultInputDevice : kAudioHardwarePropertyDefaultOutputDevice)
        var id = AudioDeviceID(0)
        var size = UInt32(MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &id) == noErr,
              id != 0 else { return nil }
        return id
    }

    /// Whether the device's output is muted (main element), when it reports it.
    static func isMuted(_ id: AudioDeviceID) -> Bool {
        var address = address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput)
        guard AudioObjectHasProperty(id, &address) else { return false }
        var muted: UInt32 = 0
        var size = UInt32(MemoryLayout<UInt32>.size)
        return AudioObjectGetPropertyData(id, &address, 0, nil, &size, &muted) == noErr && muted != 0
    }

    static func device(uid: String) -> AudioDeviceID? {
        all().first { Self.uid($0) == uid }
    }

    static func devices(input: Bool) -> [AudioDevice] {
        all().compactMap { id in
            guard hasStreams(id, input: input), let uid = uid(id) else { return nil }
            return AudioDevice(id: id, uid: uid, name: name(id) ?? uid)
        }
        .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    /// Points the I/O unit of an engine node at a device. Must happen before the engine starts.
    @discardableResult
    static func use(_ device: AudioDeviceID, on unit: AudioUnit?) -> Bool {
        guard let unit else { return false }
        var device = device
        return AudioUnitSetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0,
                                    &device, UInt32(MemoryLayout<AudioDeviceID>.size)) == noErr
    }

    /// Calls `change` on the main queue whenever devices or system defaults change.
    static func observe(_ change: @escaping @Sendable () -> Void) {
        for selector in [kAudioHardwarePropertyDevices, kAudioHardwarePropertyDefaultInputDevice,
                         kAudioHardwarePropertyDefaultOutputDevice] {
            var address = address(selector)
            AudioObjectAddPropertyListenerBlock(AudioObjectID(kAudioObjectSystemObject), &address, .main) { _, _ in
                change()
            }
        }
    }
}

enum Cameras {
    static func all() -> [AVCaptureDevice] {
        AVCaptureDevice.DiscoverySession(
            deviceTypes: [.builtInWideAngleCamera, .external, .continuityCamera],
            mediaType: .video, position: .unspecified
        ).devices
    }
    static func device(uniqueID: String) -> AVCaptureDevice? {
        AVCaptureDevice(uniqueID: uniqueID)
    }
}

/// Microphones, sound outputs and cameras currently connected, kept up to date.
@MainActor
@Observable
final class MediaDevices {
    private(set) var microphones: [AudioDevice] = []
    private(set) var speakers: [AudioDevice] = []
    private(set) var cameras: [CameraDevice] = []
    private(set) var defaultMicrophone: String?
    private(set) var defaultSpeaker: String?
    @ObservationIgnored private var observers: [NSObjectProtocol] = []

    init() {
        refresh()
        CoreAudioDevices.observe { Task { @MainActor [weak self] in self?.refresh() } }
        for name in [AVCaptureDevice.wasConnectedNotification, AVCaptureDevice.wasDisconnectedNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { _ in
                Task { @MainActor [weak self] in self?.refresh() }
            })
        }
    }

    /// The preferred camera when connected, else the system default.
    func camera(preferred: String) -> AVCaptureDevice? {
        MediaDevicePreferences.resolve(preferred, available: cameras.map(\.uniqueID))
            .flatMap(Cameras.device(uniqueID:)) ?? AVCaptureDevice.default(for: .video)
    }

    func refresh() {
        microphones = CoreAudioDevices.devices(input: true)
        speakers = CoreAudioDevices.devices(input: false)
        cameras = Cameras.all().map { CameraDevice(uniqueID: $0.uniqueID, name: $0.localizedName) }
        defaultMicrophone = CoreAudioDevices.defaultDevice(input: true).flatMap(CoreAudioDevices.name)
        defaultSpeaker = CoreAudioDevices.defaultDevice(input: false).flatMap(CoreAudioDevices.name)
    }
}
