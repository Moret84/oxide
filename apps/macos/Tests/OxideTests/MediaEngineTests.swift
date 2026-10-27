import LiveKitWebRTC
import XCTest
@testable import Oxide

@MainActor
final class MediaEngineTests: XCTestCase {
    /// The service refused our camera renegotiation, then sent its own offer: the call must go on.
    func testARemoteOfferWinsOverOurPendingOffer() async throws {
        let engine = WebRTCMediaEngine()
        let config = LKRTCConfiguration()
        config.sdpSemantics = .unifiedPlan
        let connection = try XCTUnwrap(engine.factory.peerConnection(with: config, constraints: engine.constraints, delegate: engine))
        let receiveAudio = LKRTCRtpTransceiverInit()
        receiveAudio.direction = .recvOnly
        connection.addTransceiver(of: .audio, init: receiveAudio)
        engine.callId = "call"
        engine.connection = connection
        var events: [MediaEvent] = []
        engine.onEvent = { events.append($0) }

        engine.renegotiate("call", reason: "camera on")
        try await waitUntil { connection.signalingState == .haveLocalOffer }

        let service = try XCTUnwrap(engine.factory.peerConnection(with: config, constraints: engine.constraints, delegate: nil))
        service.addTransceiver(of: .audio, init: LKRTCRtpTransceiverInit())
        let offer: LKRTCSessionDescription = try await withCheckedThrowingContinuation { continuation in
            service.offer(for: engine.constraints) { description, error in
                if let description { continuation.resume(returning: description) }
                else { continuation.resume(throwing: error ?? CancellationError()) }
            }
        }
        engine.answerRemoteOffer(callId: "call", sdp: offer.sdp)
        try await waitUntil {
            events.contains { if case .renegotiationAnswer = $0 { true } else { false } }
                || events.contains { if case .failed = $0 { true } else { false } }
        }

        XCTAssertFalse(events.contains { if case .failed = $0 { true } else { false } }, "\(events)")
        XCTAssertEqual(connection.signalingState, .stable)
        service.close()
        engine.close(callId: "call")
    }

    private func waitUntil(_ condition: () -> Bool) async throws {
        for _ in 0..<100 where !condition() {
            try await Task.sleep(for: .milliseconds(50))
        }
        XCTAssertTrue(condition(), "timed out")
    }
}
