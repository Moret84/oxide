import Foundation

/// What the media engine saw during a call, for the test call report.
struct CallDiagnostics: Equatable {
    var micPermissionDenied = false
    var cameraRequested = false
    var cameraPermissionDenied = false
    var cameraFailed = false
    /// Media reached the connected state at least once.
    var connected = false
    var disconnects = 0
    var failed = false
    var failureReason: String?
    /// `totalAudioEnergy` of the microphone source: stays ~0 when nothing is captured.
    var micEnergy = 0.0
    var audioPacketsReceived = 0
    var audioPacketsLost = 0
    /// Worst round-trip time reported by the other side, in seconds.
    var roundTrip = 0.0
    var videoFramesSent = 0

    mutating func merge(_ sample: CallDiagnostics) {
        micEnergy = max(micEnergy, sample.micEnergy)
        audioPacketsReceived = max(audioPacketsReceived, sample.audioPacketsReceived)
        audioPacketsLost = max(audioPacketsLost, sample.audioPacketsLost)
        roundTrip = max(roundTrip, sample.roundTrip)
        videoFramesSent = max(videoFramesSent, sample.videoFramesSent)
    }
}

/// Echo Bot test call report shaped like the web client's `*Errors` lists; empty means no known problem seen, not a guarantee.
struct TestCallReport: Equatable {
    var microphone: [String] = []
    var speaker: [String] = []
    var camera: [String] = []
    var network: [String] = []

    var isClean: Bool { microphone.isEmpty && speaker.isEmpty && camera.isEmpty && network.isEmpty }

    struct Context: Equatable {
        var microphones = 1
        var speakers = 1
        var mutedAtEnd = false
        var outputMuted = false
        var hasCamera = true
    }

    static func make(_ d: CallDiagnostics, _ c: Context) -> TestCallReport {
        var report = TestCallReport()
        if d.micPermissionDenied {
            report.microphone.append(L("ui.test_call_report.oxide_doesn_t_have_access_to"))
        } else if c.microphones == 0 {
            report.microphone.append(L("ui.test_call_report.no_microphone_detected"))
        } else if c.mutedAtEnd {
            report.microphone.append(L("ui.test_call_report.your_mic_was_muted"))
        } else if d.connected && d.micEnergy < 0.0001 {
            report.microphone.append(L("ui.test_call_report.the_microphone_didn_t_pick_up"))
        }
        if c.speakers == 0 {
            report.speaker.append(L("ui.test_call_report.no_audio_output_detected"))
        } else if c.outputMuted {
            report.speaker.append(L("ui.test_call_report.the_mac_s_audio_output_is"))
        }
        if d.connected && d.audioPacketsReceived == 0 {
            report.speaker.append(L("ui.test_call_report.no_sound_was_received_from_the"))
        }
        if d.cameraRequested {
            if d.cameraPermissionDenied {
                report.camera.append(L("ui.test_call_report.oxide_doesn_t_have_access_to_2"))
            } else if !c.hasCamera {
                report.camera.append(L("ui.test_call_report.no_camera_selected"))
            } else if d.cameraFailed || (d.connected && d.videoFramesSent == 0) {
                report.camera.append(L("ui.test_call_report.the_camera_didn_t_start"))
            }
        }
        if !d.connected || d.failed {
            report.network.append(d.failureReason ?? L("ui.test_call_report.couldn_t_establish_the_audio_video"))
        } else {
            if d.disconnects > 0 {
                report.network.append(L("ui.test_call_report.the_connection_was_interrupted_during_the"))
            }
            let total = d.audioPacketsReceived + d.audioPacketsLost
            let loss = total > 0 ? Double(d.audioPacketsLost) / Double(total) : 0
            if loss > 0.05 || d.roundTrip > 0.4 {
                report.network.append(L("ui.test_call_report.poor_connection_quality_high_packet_loss"))
            }
        }
        return report
    }
}
