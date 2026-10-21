import AVFoundation
import SwiftUI

/// Settings › Audio et vidéo: default call devices, with a test for each.
struct DeviceSettingsSection: View {
    @AppStorage(MediaDevicePreferences.microphoneKey) private var microphone = ""
    @AppStorage(MediaDevicePreferences.speakerKey) private var speaker = ""
    @AppStorage(MediaDevicePreferences.cameraKey) private var camera = ""
    @State private var devices = MediaDevices()
    @State private var mic = MicrophoneTester()
    @State private var sound = SpeakerTester()
    @State private var previewing = false
    @State private var cameraDenied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 16) {
                microphoneRow
                Divider()
                speakerRow
                Divider()
                cameraRow
                Divider()
                TestCallRow(onStart: {
                    mic.stop()
                    previewing = false
                })
            }
            .card(padding: 18)
            Muted(L("ui.device_settings_section.default_devices_detail"), size: 12)
        }
        .onDisappear {
            mic.stop()
            previewing = false
        }
    }

    // MARK: Microphone

    private var microphoneRow: some View {
        VStack(alignment: .leading, spacing: 10) {
            picker(L("ui.device_settings_section.microphone"), symbol: "mic", selection: $microphone,
                   options: devices.microphones.map { ($0.uid, $0.name) },
                   systemName: devices.defaultMicrophone)
                .onChange(of: microphone) { _, _ in restartMicrophone() }
            HStack(spacing: 12) {
                LevelMeter(tester: mic)
                    .frame(height: 8)
                Button(mic.running ? L("ui.device_settings_section.stop") : L("ui.device_settings_section.test_mic")) {
                    if mic.running { mic.stop() } else { Task { await mic.start() } }
                }
                .buttonStyle(OutlineButtonStyle())
                Button(mic.recording ? L("ui.device_settings_section.recording") : mic.playingBack ? L("ui.device_settings_section.playing") : L("ui.device_settings_section.hear_yourself_5_s")) {
                    Task { await mic.echo() }
                }
                .buttonStyle(OutlineButtonStyle())
                .disabled(mic.recording || mic.playingBack)
                .help(L("ui.device_settings_section.records_5_seconds_then_plays_them"))
            }
            if let error = mic.error { problem(error, settings: .audio) }
        }
    }

    private func restartMicrophone() {
        guard mic.running else { return }
        mic.stop()
        Task { await mic.start() }
    }

    // MARK: Speaker

    private var speakerRow: some View {
        VStack(alignment: .leading, spacing: 10) {
            picker(L("ui.device_settings_section.speaker"), symbol: "speaker.wave.2", selection: $speaker,
                   options: devices.speakers.map { ($0.uid, $0.name) },
                   systemName: devices.defaultSpeaker)
            HStack(spacing: 12) {
                Button(sound.playing ? L("ui.device_settings_section.playing") : L("ui.device_settings_section.test_speaker")) { sound.play() }
                    .buttonStyle(OutlineButtonStyle())
                    .disabled(sound.playing)
                if let error = sound.error { Text(error).font(.figtree(12)).foregroundStyle(Theme.danger) }
            }
        }
    }

    // MARK: Camera

    private var cameraRow: some View {
        VStack(alignment: .leading, spacing: 10) {
            picker(L("ui.device_settings_section.camera"), symbol: "video", selection: $camera,
                   options: devices.cameras.map { ($0.uniqueID, $0.name) },
                   systemName: AVCaptureDevice.default(for: .video)?.localizedName)
            HStack(alignment: .top, spacing: 12) {
                ZStack {
                    RoundedRectangle(cornerRadius: 10).fill(Theme.input)
                    if previewing {
                        CameraPreview(device: selectedCamera)
                            .clipShape(RoundedRectangle(cornerRadius: 10))
                    } else {
                        Image(systemName: "video.slash").font(.system(size: 22)).foregroundStyle(Theme.subtle)
                    }
                }
                .frame(width: 256, height: 144)
                Button(previewing ? L("ui.device_settings_section.stop_preview") : L("ui.device_settings_section.preview")) {
                    if previewing { previewing = false } else {
                        Task {
                            cameraDenied = !(await MediaPermissions.request(.video))
                            previewing = !cameraDenied
                        }
                    }
                }
                .buttonStyle(OutlineButtonStyle())
            }
            if cameraDenied {
                problem(L("ui.device_settings_section.camera_access_is_denied_allow_oxide"),
                        settings: .video)
            }
        }
    }

    private var selectedCamera: AVCaptureDevice? { devices.camera(preferred: camera) }

    // MARK: Shared

    private func picker(_ title: String, symbol: String, selection: Binding<String>,
                        options: [(id: String, name: String)], systemName: String?) -> some View {
        HStack(spacing: 10) {
            Image(systemName: symbol).frame(width: 18).foregroundStyle(Theme.muted)
            Text(title).font(.figtree(13, .semibold)).frame(width: 96, alignment: .leading)
            Picker(title, selection: selection) {
                Text(systemName.map { L("ui.device_settings_section.system_default", ["device": $0]) } ?? L("ui.device_settings_section.system_default_2")).tag("")
                ForEach(options, id: \.id) { option in Text(option.name).tag(option.id) }
                if !selection.wrappedValue.isEmpty && !options.contains(where: { $0.id == selection.wrappedValue }) {
                    Text(L("ui.device_settings_section.device_not_connected")).tag(selection.wrappedValue)
                }
            }
            .labelsHidden()
            .frame(maxWidth: 360)
        }
    }

    private func problem(_ text: String, settings type: AVMediaType) -> some View {
        HStack(spacing: 8) {
            Text(text).font(.figtree(12)).foregroundStyle(Theme.danger)
            Button(L("ui.device_settings_section.open_settings")) { NSWorkspace.shared.open(MediaPermissions.settingsURL(type)) }
                .buttonStyle(.link)
        }
    }
}

/// Reads the level itself, so only the meter is redrawn at its rate, not the whole section.
private struct LevelMeter: View {
    let tester: MicrophoneTester
    var body: some View {
        let level = tester.level, active = tester.running
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                Capsule().fill(Theme.input)
                Capsule()
                    .fill(level > 0.85 ? Theme.danger : Theme.accent)
                    .frame(width: geometry.size.width * CGFloat(active ? level : 0))
                    .animation(.linear(duration: 0.08), value: level)
            }
        }
        .frame(minWidth: 120)
        .accessibilityLabel(L("ui.device_settings_section.mic_level"))
        .accessibilityValue(L("ui.device_settings_section.text", ["percent": Int(level * 100)]))
    }
}

enum TestCall {
    /// Optional override of the Echo Bot MRI (`defaults write fr.neicureuil.oxide call.echoBotMri 28:…`).
    static var botOverride: String? {
        UserDefaults.standard.string(forKey: "call.echoBotMri").flatMap { $0.isEmpty ? nil : $0 }
    }
}

/// Settings › Audio et vidéo › Appel de test: a real call with the Echo Bot, then a report.
private struct TestCallRow: View {
    @Environment(Store.self) private var store
    let onStart: () -> Void

    /// The core's status text once registered for calls.
    private var ready: Bool {
        store.calls?.realtime == L("core.calls.ready_to_receive_calls")
    }
    private var busy: Bool { store.inCall }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Image(systemName: "phone.badge.waveform").frame(width: 18).foregroundStyle(Theme.muted)
                Text(L("ui.device_settings_section.test_call")).font(.figtree(13, .semibold)).frame(width: 96, alignment: .leading)
                Button(L("ui.device_settings_section.make_a_test_call")) {
                    onStart()
                    store.send(.call(action: .testCall(botMri: TestCall.botOverride)))
                }
                .buttonStyle(OutlineButtonStyle())
                .disabled(!ready || busy)
            }
            Muted(hint, size: 12)
        }
    }

    private var hint: String {
        if !ready {
            let status = store.calls?.realtime ?? ""
            return status.isEmpty ? L("ui.device_settings_section.connecting_to_the_calling_service") : status
        }
        if busy { return L("ui.device_settings_section.a_call_is_already_in_progress") }
        return L("ui.device_settings_section.calls_the_teams_echo_bot_it")
    }
}
