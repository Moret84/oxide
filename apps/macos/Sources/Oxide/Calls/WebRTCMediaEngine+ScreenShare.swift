import CoreGraphics
import LiveKitWebRTC

extension WebRTCMediaEngine {
    func setScreenShare(callId: String, on: Bool) {
        guard self.callId == callId, let connection, connection.signalingState == .stable else {
            if on { onEvent?(.screenShareStopped(callId: callId, reason: L("ui.media_engine.the_media_connection_isn_t_ready"))) }
            return
        }
        if on {
            guard CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() else {
                onEvent?(.screenShareStopped(callId: callId, reason:
                    L("ui.media_engine.allow_oxide_in_system_settings_privacy")))
                return
            }
            stopScreenCapture()
            let source = factory.videoSource(forScreenCast: true)
            let capturer = chosenScreen.map { LKRTCDesktopCapturer(source: $0.source, delegate: self, capture: source) }
                ?? LKRTCDesktopCapturer(defaultScreen: self, capture: source)
            chosenScreen = nil
            capturer.startCapture(withFPS: 15)
            screenCapturer = capturer
            let track = factory.videoTrack(with: source, trackId: "screen0")
            screenTrack = track
            measure(track)
            // Send-only like the web client: offered both ways, the mixer answers `sendonly` and never takes our screen.
            if let line = videoLine(sharing: true) {
                line.sender.track = track
                line.setDirection(.sendOnly, error: nil)
                sharingTransceiver = line
            } else {
                let options = LKRTCRtpTransceiverInit()
                options.direction = .sendOnly
                options.streamIds = ["local"]
                sharingTransceiver = connection.addTransceiver(with: track, init: options)
            }
        } else {
            stopScreenCapture()
            guard let line = videoLine(sharing: true) else { return }
            line.sender.track = nil
            line.setDirection(.recvOnly, error: nil)
        }
        renegotiate(callId, reason: on ? "screen share on" : "screen share off")
    }

    func stopScreenCapture() {
        if let screenCapturer {
            screenCapturer.stopCapture()
            releaseNativeSelfReference(screenCapturer)
        }
        screenCapturer = nil
        if let screenTrack, let probe = probes.removeValue(forKey: screenTrack.trackId) { screenTrack.remove(probe) }
        screenTrack = nil
    }
}

extension WebRTCMediaEngine: LKRTCDesktopCapturerDelegate {
    nonisolated func didSourceCaptureStart(_ capturer: LKRTCDesktopCapturer) {}
    nonisolated func didSourceCapturePaused(_ capturer: LKRTCDesktopCapturer) {}
    nonisolated func didSourceCaptureStop(_ capturer: LKRTCDesktopCapturer) {}
    nonisolated func didSourceCaptureError(_ capturer: LKRTCDesktopCapturer) {
        Task { @MainActor in
            guard capturer === self.screenCapturer, let callId = self.callId else { return }
            self.setScreenShare(callId: callId, on: false)
            self.onEvent?(.screenShareStopped(callId: callId, reason: L("ui.media_engine.screen_sharing_stopped")))
        }
    }
}
