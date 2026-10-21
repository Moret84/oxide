import XCTest
import OxideCore
@testable import Oxide

final class CallsTests: XCTestCase {
    private var paris: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "Europe/Paris")!
        return c
    }
    private func log(_ id: String, _ start: String, outcome: CallOutcome = .answered,
                     direction: CallDirectionView = .incoming, duration: Int64? = nil, group: Bool = false) -> CallLogView {
        CallLogView(id: id, name: id, peer: "", direction: direction, outcome: outcome,
                    startedAt: start, durationSecs: duration, group: group, canCallBack: true)
    }

    @MainActor
    func testScreenShareSwitchesLayoutUntilItEndsUnlessOneIsPicked() {
        let saved = UserDefaults.standard.string(forKey: "call.layout")
        defer { UserDefaults.standard.set(saved, forKey: "call.layout") }
        let layout = CallLayoutState()
        layout.pick(.speaker)
        layout.screenShared(true)
        XCTAssertEqual(layout.current, .sharing)
        layout.screenShared(false)
        XCTAssertEqual(layout.current, .speaker, "back to the layout before the share")

        layout.screenShared(true)
        layout.pick(.focus)
        layout.screenShared(false)
        XCTAssertEqual(layout.current, .focus, "a layout picked during the share stays")
        XCTAssertEqual(CallLayoutState().current, .focus, "remembered for the next call")

        layout.togglePin("8:orgid:bob")
        XCTAssertEqual(layout.pinned, "8:orgid:bob")
        layout.togglePin("8:orgid:bob")
        XCTAssertNil(layout.pinned)
    }

    func testHistoryIsGroupedByLocalDayKeepingOrder() {
        let now = CalendarDates.date("2026-10-15T10:00:00Z")!
        let logs = [
            log("a", "2026-10-15T08:00:00Z"),
            log("b", "2026-10-14T21:30:00Z"),
            // 23:30 UTC on the 14th is already the 15th in Paris.
            log("c", "2026-10-14T22:30:00Z"),
            log("d", "2026-10-10T08:00:00Z"),
            log("e", "not a date"),
        ]
        let days = CallsFormat.days(logs, date: \.startedAt, now: now, calendar: paris)
        XCTAssertEqual(days.map(\.title).prefix(2), ["Aujourd’hui", "Hier"])
        XCTAssertEqual(days[0].items.map(\.id), ["a", "c"])
        XCTAssertEqual(days[1].items.map(\.id), ["b"])
        XCTAssertEqual(days.last?.title, "Date inconnue")
        XCTAssertEqual(days.count, 4)
    }

    func testDurationsAndSummariesReadNaturally() {
        XCTAssertEqual(CallsFormat.duration(42), "42 s")
        XCTAssertEqual(CallsFormat.duration(120), "2 min")
        XCTAssertEqual(CallsFormat.duration(125), "2 min 05 s")
        XCTAssertEqual(CallsFormat.duration(3_725), "1 h 02 min")
        XCTAssertEqual(CallsFormat.summary(log("x", "", outcome: .missed)), "Appel manqué")
        XCTAssertEqual(CallsFormat.summary(log("x", "", outcome: .declined, group: true)), "Appel de groupe refusé")
        XCTAssertEqual(CallsFormat.summary(log("x", "", direction: .outgoing, duration: 65)), "Appel sortant · 1 min 05 s")
        XCTAssertEqual(CallsFormat.symbol(log("x", "", outcome: .missed)), "phone.down")
        XCTAssertEqual(CallsFormat.symbol(log("x", "", direction: .outgoing)), "phone.arrow.up.right")
    }

    func testSavedDevicesFallBackToTheSystemDefaultWhenDisconnected() {
        XCTAssertEqual(MediaDevicePreferences.resolve("usb-mic", available: ["built-in", "usb-mic"]), "usb-mic")
        XCTAssertNil(MediaDevicePreferences.resolve("usb-mic", available: ["built-in"]))
        XCTAssertNil(MediaDevicePreferences.resolve("", available: ["built-in"]))
        XCTAssertNil(MediaDevicePreferences.resolve(nil, available: ["built-in"]))
    }

    func testCallClockAndRoles() {
        let start: Int64 = 1_000_000
        XCTAssertEqual(CallClock.elapsed(since: start, now: Date(timeIntervalSince1970: 1_000 + 65)), "01:05")
        XCTAssertEqual(CallClock.elapsed(since: start, now: Date(timeIntervalSince1970: 1_000 + 3_725)), "1:02:05")
        XCTAssertEqual(CallClock.elapsed(since: start, now: Date(timeIntervalSince1970: 0)), "00:00")
        XCTAssertEqual(CallClock.role("organizer"), "Organisateur")
        XCTAssertNil(CallClock.role(""))
    }

    func testTestCallReportListsOnlyDetectedProblems() {
        var ok = CallDiagnostics(connected: true, micEnergy: 0.3, audioPacketsReceived: 400)
        XCTAssertTrue(TestCallReport.make(ok, .init()).isClean)

        ok.cameraRequested = true
        XCTAssertEqual(TestCallReport.make(ok, .init()).camera, ["La caméra n’a pas démarré."])
        let denied = CallDiagnostics(cameraRequested: true, cameraPermissionDenied: true, connected: true,
                                     micEnergy: 1, audioPacketsReceived: 1)
        XCTAssertEqual(TestCallReport.make(denied, .init()).camera, ["Oxide n’a pas accès à la caméra."])

        let silent = CallDiagnostics(connected: true, audioPacketsReceived: 0)
        let report = TestCallReport.make(silent, .init(outputMuted: true))
        XCTAssertEqual(report.microphone, ["Le micro n’a capté aucun son."])
        XCTAssertEqual(report.speaker, ["La sortie audio du Mac est coupée.", "Aucun son n’a été reçu de l’Echo Bot."])
        XCTAssertEqual(TestCallReport.make(silent, .init(mutedAtEnd: true)).microphone, ["Votre micro était coupé."])
        XCTAssertEqual(TestCallReport.make(CallDiagnostics(micPermissionDenied: true), .init(microphones: 0)).microphone,
                       ["Oxide n’a pas accès au micro."])

        XCTAssertEqual(TestCallReport.make(CallDiagnostics(), .init()).network,
                       ["Impossible d’établir la connexion audio/vidéo."])
        XCTAssertEqual(TestCallReport.make(CallDiagnostics(failureReason: "Réponse média refusée"), .init()).network,
                       ["Réponse média refusée"])
        let lossy = CallDiagnostics(connected: true, disconnects: 1, micEnergy: 1,
                                    audioPacketsReceived: 90, audioPacketsLost: 10)
        XCTAssertEqual(TestCallReport.make(lossy, .init()).network,
                       ["La connexion a été interrompue pendant l’appel.",
                        "Connexion de mauvaise qualité (pertes ou latence élevées)."])
    }
}
