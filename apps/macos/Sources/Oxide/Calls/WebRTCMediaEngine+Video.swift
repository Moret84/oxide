import AVFoundation
import LiveKitWebRTC
import os

extension WebRTCMediaEngine {
    /// The camera chosen in Settings, capturing.
    func makeCameraTrack() -> LKRTCVideoTrack? {
        guard let device = MediaDevicePreferences.resolvedCamera() else { return nil }
        let source = factory.videoSource()
        let capturer = LKRTCCameraVideoCapturer(delegate: source)
        if let format = Self.format(for: device) {
            let fps = min(30, Int(format.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 30))
            capturer.startCapture(with: device, format: format, fps: fps) { [weak self] error in
                guard error != nil else { return }
                Task { @MainActor in self?.diagnostics.cameraFailed = true }
            }
        }
        self.capturer = capturer
        let track = factory.videoTrack(with: source, trackId: "video0")
        measure(track)
        return track
    }

    func stopCamera() {
        capturer?.stopCapture()
        capturer = nil
    }

    /// Mid-call, the camera line changes direction and renegotiates, like the web client's `startRenegotiationAsync`.
    func setCamera(callId: String, on: Bool) {
        guard self.callId == callId else { return }
        // Before the first answer, or with a negotiation in flight: only the track changes.
        guard connection?.remoteDescription != nil, connection?.signalingState == .stable else {
            localVideo?.isEnabled = on
            return
        }
        Task {
            if on {
                guard await MediaPermissions.request(.video) else {
                    diagnostics.cameraPermissionDenied = true
                    return
                }
            }
            guard self.callId == callId, let line = videoLine(sharing: false) else { return }
            if on {
                if localVideo == nil { localVideo = makeCameraTrack() }
                guard let localVideo else { return }
                localVideo.isEnabled = true
                line.sender.track = localVideo
                line.setDirection(.sendRecv, error: nil)
            } else {
                stopCamera()
                line.sender.track = nil
                line.setDirection(.recvOnly, error: nil)
                localVideo = nil
            }
            cameraTransceiver = line
            renegotiate(callId, reason: on ? "camera on" : "camera off")
        }
    }

    /// The remembered video line, else by position (an incoming call's lines come from the caller's offer).
    func videoLine(sharing: Bool) -> LKRTCRtpTransceiver? {
        let remembered = sharing ? sharingTransceiver : cameraTransceiver
        if let remembered, !remembered.isStopped { return remembered }
        let lines = connection?.transceivers.filter { $0.mediaType == .video && !$0.isStopped } ?? []
        guard lines.count > (sharing ? 1 : 0) else { return nil }
        return sharing ? lines.last : lines.first
    }

    /// 720p when available, else the largest format up to 1280 px wide.
    private static func format(for device: AVCaptureDevice) -> AVCaptureDevice.Format? {
        let formats = LKRTCCameraVideoCapturer.supportedFormats(for: device)
        func width(_ f: AVCaptureDevice.Format) -> Int32 { CMVideoFormatDescriptionGetDimensions(f.formatDescription).width }
        return formats.first { width($0) == 1280 }
            ?? formats.filter { width($0) <= 1280 }.max { width($0) < width($1) }
            ?? formats.first
    }

    /// Follows a local track's frame size, to scale its encoding to the mixer's bounds.
    func measure(_ track: LKRTCVideoTrack) {
        let id = track.trackId
        if let old = probes[id] { track.remove(old) }
        let probe = FrameSizeProbe { [weak self] size in
            guard let self else { return }
            self.frameSizes[id] = size
            self.reapplySendLimits(trackId: id)
        }
        track.add(probe)
        probes[id] = probe
    }

    private func line(mid: String) -> LKRTCRtpTransceiver? {
        connection?.transceivers.first { $0.mid == mid && !$0.isStopped }
    }

    private func reapplySendLimits(trackId: String) {
        guard let line = connection?.transceivers.first(where: { $0.sender.track?.trackId == trackId }) else { return }
        if let limits = sendLimits[line.mid] {
            apply(limits, to: line, keyframe: false)
        } else if trackId == screenTrack?.trackId {
            // Before the mixer says anything, the sharing line's own bounds (`applySenderParamsBeforeStart`).
            apply(SendLimits(mid: line.mid, sharing: true, maxFs: 8160, maxFps: 15, maxBitrate: 0, keyframe: true),
                  to: line, keyframe: true)
        }
    }

    func applySendLimits(callId: String, _ limits: SendLimits) {
        guard self.callId == callId else { return }
        sendLimits[limits.mid] = limits
        guard let line = line(mid: limits.mid), line.sender.track != nil else { return }
        apply(limits, to: line, keyframe: limits.keyframe)
    }

    /// Web client's `updateEncodingsWithCaps` / `generateKeyFrameIfNeeded`: screens capped at 8160 fs and 15 fps; keyframe via a brief 10 % upscale.
    private func apply(_ limits: SendLimits, to line: LKRTCRtpTransceiver, keyframe: Bool) {
        guard let trackId = line.sender.track?.trackId, let size = frameSizes[trackId], size.width > 0 else { return }
        let maxFs = Double(limits.sharing ? min(limits.maxFs, 8160) : limits.maxFs)
        let fps = limits.sharing ? min(limits.maxFps, 15) : limits.maxFps
        let scale = max(1, (size.width * size.height / (maxFs * 256)).squareRoot())
        let set = { (scale: Double) in
            let parameters = line.sender.parameters
            for encoding in parameters.encodings {
                encoding.scaleResolutionDownBy = NSNumber(value: scale)
                encoding.maxFramerate = NSNumber(value: fps)
                if limits.maxBitrate > 0 { encoding.maxBitrateBps = NSNumber(value: limits.maxBitrate) }
            }
            line.sender.parameters = parameters
        }
        callTrace("send limits mid=\(line.mid) \(Int(size.width))x\(Int(size.height)) fs=\(Int(maxFs)) scale=\(String(format: "%.2f", scale)) fps=\(fps) br=\(limits.maxBitrate) keyframe=\(keyframe)")
        guard keyframe else {
            set(scale)
            return
        }
        set(scale * 1.1)
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(100))
            guard self?.connection != nil, !line.isStopped else { return }
            set(scale)
        }
    }
}

/// Reports the size of a local track's frames when it changes (a shared window can resize).
final class FrameSizeProbe: NSObject, LKRTCVideoRenderer, @unchecked Sendable {
    private let changed: @MainActor (CGSize) -> Void
    private let last = OSAllocatedUnfairLock(initialState: CGSize.zero)

    init(changed: @escaping @MainActor (CGSize) -> Void) { self.changed = changed }

    func setSize(_ size: CGSize) {}

    func renderFrame(_ frame: LKRTCVideoFrame?) {
        guard let frame else { return }
        let size = CGSize(width: Int(frame.width), height: Int(frame.height))
        let isNew = last.withLock { last in
            defer { last = size }
            return last != size
        }
        guard isNew else { return }
        Task { @MainActor [changed] in changed(size) }
    }
}
