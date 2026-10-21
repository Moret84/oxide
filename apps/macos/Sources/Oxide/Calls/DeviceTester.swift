import AVFoundation
import AppKit
import Observation
import os
import SwiftUI

/// Live input level of the chosen microphone, and a short record-and-play-back check.
@MainActor
@Observable
final class MicrophoneTester {
    /// 0…1, smoothed.
    private(set) var level: Float = 0
    private(set) var running = false
    private(set) var recording = false
    private(set) var playingBack = false
    private(set) var error: String?
    @ObservationIgnored private var engine: AVAudioEngine?
    @ObservationIgnored private var playback: AVAudioEngine?
    /// Shared with the audio thread: the loudest buffer since the last level update, and the recording.
    @ObservationIgnored private let tap = OSAllocatedUnfairLock(uncheckedState: TapState())

    private struct TapState {
        var peak: Float = 0
        var lastPublish: UInt64 = 0
        var recording = false
        var recorded: [AVAudioPCMBuffer] = []
    }

    /// The meter is refreshed at most this often (~30 Hz), however small the audio buffers.
    nonisolated private static let publishInterval: UInt64 = 33_000_000

    func start() async {
        guard !running else { return }
        guard await MediaPermissions.request(.audio) else {
            error = L("ui.device_tester.microphone_access_is_denied_allow_oxide")
            return
        }
        let engine = AVAudioEngine()
        if let device = MediaDevicePreferences.resolvedMicrophone() {
            CoreAudioDevices.use(device, on: engine.inputNode.audioUnit)
        }
        let format = engine.inputNode.inputFormat(forBus: 0)
        guard format.channelCount > 0, format.sampleRate > 0 else {
            error = L("ui.device_tester.this_microphone_isn_t_sending_any")
            return
        }
        tap.withLockUnchecked { $0 = TapState() }
        let tap = tap
        engine.inputNode.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in
            let rms = Self.rms(buffer)
            let now = DispatchTime.now().uptimeNanoseconds
            let peak: Float? = tap.withLockUnchecked { state in
                if state.recording, let copy = Self.copy(buffer) { state.recorded.append(copy) }
                state.peak = max(state.peak, rms)
                guard now &- state.lastPublish >= Self.publishInterval else { return nil }
                state.lastPublish = now
                defer { state.peak = 0 }
                return state.peak
            }
            guard let peak else { return }
            Task { @MainActor [weak self] in self?.receive(rms: peak) }
        }
        do {
            try engine.start()
            self.engine = engine
            running = true
            error = nil
        } catch {
            engine.inputNode.removeTap(onBus: 0)
            self.error = L("ui.device_tester.couldn_t_open_the_microphone")
        }
    }

    func stop() {
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        engine = nil
        playback?.stop()
        playback = nil
        running = false
        recording = false
        playingBack = false
        level = 0
        tap.withLockUnchecked { $0 = TapState() }
    }

    /// Records five seconds, then plays them on the chosen output.
    func echo() async {
        if !running { await start() }
        guard running, !recording, !playingBack else { return }
        tap.withLockUnchecked { $0.recorded = []; $0.recording = true }
        recording = true
        try? await Task.sleep(for: .seconds(5))
        guard recording else { return }
        recording = false
        let recorded = tap.withLockUnchecked { state in
            state.recording = false
            defer { state.recorded = [] }
            return state.recorded
        }
        play(recorded)
    }

    private func receive(rms: Float) {
        guard running else { return }
        // -50 dBFS (silence) … 0 dBFS (full scale), with a fast attack and slow release.
        let db = 20 * log10(max(rms, 1e-6))
        let target = max(0, min(1, (db + 50) / 50))
        level = target > level ? target : level * 0.85 + target * 0.15
    }

    private func play(_ buffers: [AVAudioPCMBuffer]) {
        guard let format = buffers.first?.format else { return }
        let (engine, player) = speakerEngine(format: format)
        do { try engine.start() } catch {
            self.error = L("ui.device_tester.couldn_t_play_back_the_recording")
            return
        }
        playback = engine
        playingBack = true
        for (index, buffer) in buffers.enumerated() {
            let last = index == buffers.count - 1
            player.scheduleBuffer(buffer) { [weak self] in
                guard last else { return }
                Task { @MainActor [weak self] in
                    self?.playingBack = false
                    self?.playback?.stop()
                    self?.playback = nil
                }
            }
        }
        player.play()
    }

    nonisolated private static func rms(_ buffer: AVAudioPCMBuffer) -> Float {
        guard let data = buffer.floatChannelData, buffer.frameLength > 0 else { return 0 }
        let samples = UnsafeBufferPointer(start: data[0], count: Int(buffer.frameLength))
        let sum = samples.reduce(Float(0)) { $0 + $1 * $1 }
        return (sum / Float(buffer.frameLength)).squareRoot()
    }

    nonisolated private static func copy(_ buffer: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
        guard let copy = AVAudioPCMBuffer(pcmFormat: buffer.format, frameCapacity: buffer.frameLength),
              let source = buffer.floatChannelData, let target = copy.floatChannelData else { return nil }
        copy.frameLength = buffer.frameLength
        for channel in 0..<Int(buffer.format.channelCount) {
            target[channel].update(from: source[channel], count: Int(buffer.frameLength))
        }
        return copy
    }
}

/// An engine whose player node plays `format` on the chosen output; start it before scheduling.
private func speakerEngine(format: AVAudioFormat) -> (AVAudioEngine, AVAudioPlayerNode) {
    let engine = AVAudioEngine()
    let player = AVAudioPlayerNode()
    engine.attach(player)
    engine.connect(player, to: engine.mainMixerNode, format: format)
    if let speaker = MediaDevicePreferences.resolvedSpeaker() {
        CoreAudioDevices.use(speaker, on: engine.outputNode.audioUnit)
    }
    return (engine, player)
}

/// Plays a short two-tone chime on the chosen output.
@MainActor
@Observable
final class SpeakerTester {
    private(set) var playing = false
    private(set) var error: String?
    @ObservationIgnored private var engine: AVAudioEngine?

    func play() {
        guard !playing else { return }
        let format = AVAudioFormat(standardFormatWithSampleRate: 44_100, channels: 2)!
        let (engine, player) = speakerEngine(format: format)
        guard let buffer = Self.chime(format) else { return }
        do { try engine.start() } catch {
            self.error = L("ui.device_tester.couldn_t_use_this_audio_output")
            return
        }
        self.engine = engine
        playing = true
        error = nil
        player.scheduleBuffer(buffer) { [weak self] in
            Task { @MainActor [weak self] in
                self?.engine?.stop()
                self?.engine = nil
                self?.playing = false
            }
        }
        player.play()
    }

    private static func chime(_ format: AVAudioFormat) -> AVAudioPCMBuffer? {
        let rate = format.sampleRate
        let frames = AVAudioFrameCount(rate * 1.2)
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames),
              let channels = buffer.floatChannelData else { return nil }
        buffer.frameLength = frames
        for i in 0..<Int(frames) {
            let t = Double(i) / rate
            let (frequency, start) = t < 0.6 ? (660.0, 0.0) : (880.0, 0.6)
            let local = t - start
            let envelope = min(local / 0.02, 1) * exp(-local * 4)
            let sample = Float(0.25 * envelope * sin(2 * .pi * frequency * t))
            channels[0][i] = sample
            channels[1][i] = sample
        }
        return buffer
    }
}

/// Live preview of a camera, mirrored like a video call self-view.
struct CameraPreview: NSViewRepresentable {
    let device: AVCaptureDevice?

    final class PreviewView: NSView {
        let session = AVCaptureSession()
        private let layerPreview: AVCaptureVideoPreviewLayer
        private let queue = DispatchQueue(label: "camera-preview")
        private(set) var current: String?

        override init(frame: NSRect) {
            layerPreview = AVCaptureVideoPreviewLayer(session: session)
            super.init(frame: frame)
            wantsLayer = true
            layer = CALayer()
            layer?.backgroundColor = NSColor.black.cgColor
            layerPreview.videoGravity = .resizeAspectFill
            layer?.addSublayer(layerPreview)
        }
        required init?(coder: NSCoder) { fatalError() }
        override func layout() {
            super.layout()
            layerPreview.frame = bounds
        }

        func show(_ device: AVCaptureDevice?) {
            guard device?.uniqueID != current else { return }
            current = device?.uniqueID
            let session = session
            queue.async {
                session.beginConfiguration()
                session.inputs.forEach(session.removeInput)
                if let device, let input = try? AVCaptureDeviceInput(device: device), session.canAddInput(input) {
                    session.addInput(input)
                }
                session.commitConfiguration()
                if !session.isRunning && !session.inputs.isEmpty { session.startRunning() }
            }
            if let connection = layerPreview.connection, connection.isVideoMirroringSupported {
                connection.automaticallyAdjustsVideoMirroring = false
                connection.isVideoMirrored = true
            }
        }

        func stop() {
            let session = session
            queue.async { session.stopRunning() }
        }
    }

    func makeNSView(context: Context) -> PreviewView {
        let view = PreviewView()
        view.show(device)
        return view
    }
    func updateNSView(_ view: PreviewView, context: Context) {
        view.show(device)
    }
    static func dismantleNSView(_ view: PreviewView, coordinator: ()) {
        view.stop()
    }
}
