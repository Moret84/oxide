import OxideCore
import XCTest
@testable import Oxide

final class FeedTests: XCTestCase {
    private func message(_ id: String, day: String = "15/10/2026", text: String = "") -> MessageView {
        MessageView(
            id: id, sender: "Alice", senderId: "8:orgid:alice", senderAvatarShape: .round, mine: false, time: "10:00", day: day,
            subject: nil, importance: .standard, deleted: false, editing: false,
            runs: [RunView(text: text, link: nil, bold: false, italic: false, code: false, underline: false,
                           strike: false, color: nil, highlight: nil, size: 0, mention: false)],
            blocks: [], images: [], files: [], linkPreview: nil, reactions: [], pending: false, canReact: true,
            editable: false, receipt: nil
        )
    }

    func testListsKeepUnchangedViewsAndDropTheOnesThatLeft() {
        var list = FeedList<MessageView>()
        list.apply(["a", "b"], [message("a"), message("b")], stamp: 1, id: \.id)
        list.apply(nil, [message("b", text: "modifié")], stamp: 2, id: \.id)
        XCTAssertEqual(list.ordered.map(\.stamp), [1, 2])
        XCTAssertEqual(list.ordered.last?.view.runs.first?.text, "modifié")
        list.apply(["b"], [], stamp: 3, id: \.id)
        XCTAssertEqual(list.ordered.map(\.view.id), ["b"])
        // A view the core sends again after it left the list comes back with it.
        list.apply(["a", "b"], [message("a")], stamp: 4, id: \.id)
        XCTAssertEqual(list.ordered.map(\.stamp), [4, 2])
    }

    func testRowsCompareByStampAndDaySeparatorsKeepTheirIdWhenOlderMessagesLoad() {
        let recent = FeedItem.messages([(message("b", day: "16/10/2026"), 1)], bubbles: true)
        let older = FeedItem.messages(
            [(message("a", day: "16/10/2026"), 2), (message("b", day: "16/10/2026"), 1)], bubbles: true
        )
        XCTAssertEqual(recent.first?.id, "day:16/10/2026")
        XCTAssertEqual(older.first?.id, recent.first?.id, "the separator row is kept, not rebuilt")
        XCTAssertEqual(older.last, recent.last)
        XCTAssertNotEqual(
            FeedItem.message(message("b"), bubble: true, stamp: 1),
            FeedItem.message(message("b"), bubble: true, stamp: 2)
        )
        XCTAssertEqual(FeedRows(revision: 3, items: recent), FeedRows(revision: 3, items: older))
    }

    func testNewMessagesSeparatorSitsAfterTheDayAndBeforeTheFirstUnread() {
        let items = FeedItem.messages(
            [(message("a"), 1), (message("b", day: "16/10/2026"), 1), (message("c", day: "16/10/2026"), 1)],
            bubbles: true, firstUnread: "b"
        )
        XCTAssertEqual(items.map(\.id), ["day:15/10/2026", "message:a", "day:16/10/2026", "unread", "message:b", "message:c"])
        XCTAssertFalse(FeedItem.messages([(message("a"), 1)], bubbles: true).contains(.unread))
    }

    func testUpInAnEmptyComposerEditsMyLatestMessageStillEditable() {
        func mine(_ id: String, editable: Bool = true, deleted: Bool = false) -> MessageView {
            var view = message(id)
            view.mine = true
            view.editable = editable
            view.deleted = deleted
            return view
        }
        let rows = { (views: [MessageView]) in
            FeedRows(revision: 1, items: FeedItem.messages(views.map { ($0, 1) }, bubbles: true))
        }
        XCTAssertEqual(Store.lastEditable(in: rows([mine("a"), message("b"), mine("c", deleted: true)]))?.id, "a")
        XCTAssertNil(Store.lastEditable(in: rows([mine("a"), mine("b", editable: false)])))
        XCTAssertNil(Store.lastEditable(in: rows([message("a")])))
    }

    func testScrollingUpToTheEarlierRowLoadsItUnlessALoadIsRunning() {
        let more = FeedItem.more(label: "", action: .moreFeed, loading: false)
        let feed = [more, .note("x"), .note("y")]
        XCTAssertEqual(FeedItem.earlierAction(feed, visible: NSRange(location: 0, length: 2)), .moreFeed)
        XCTAssertNil(FeedItem.earlierAction(feed, visible: NSRange(location: 1, length: 2)), "not on screen")
        XCTAssertNil(FeedItem.earlierAction(feed, visible: NSRange(location: NSNotFound, length: 0)), "nothing laid out")
        let loading = FeedItem.more(label: "", action: .moreFeed, loading: true)
        XCTAssertNil(FeedItem.earlierAction([loading, .note("x")], visible: NSRange(location: 0, length: 2)))
        XCTAssertNil(FeedItem.earlierAction([.note("x")], visible: NSRange(location: 0, length: 1)), "no history left")
    }
    func testOnlyAGestureStartingAtTheTopLoadsOneEarlierPage() {
        var gate = EarlierLoadGate()
        gate.gestureBegan(rowOnScreen: false)
        XCTAssertNil(gate.pass(.moreFeed), "the gesture that reaches the row only shows it")
        gate.gestureBegan(rowOnScreen: true)
        XCTAssertNil(gate.pass(nil), "row gone off screen")
        XCTAssertEqual(gate.pass(.moreFeed), .moreFeed)
        XCTAssertNil(gate.pass(.moreFeed), "one page per gesture")
    }
    func testAMouseWheelLoadsOneEarlierPagePerSpinStartingAtTheTop() {
        var gate = EarlierLoadGate()
        gate.wheelTick(at: 10, rowOnScreen: false)
        for tick in 1...100 { gate.wheelTick(at: 10 + Double(tick) * 0.01, rowOnScreen: true) }
        XCTAssertNil(gate.pass(.moreFeed), "a free-spinning wheel reaching the row is one gesture")
        gate.wheelTick(at: 11 + EarlierLoadGate.wheelPause, rowOnScreen: true)
        XCTAssertEqual(gate.pass(.moreFeed), .moreFeed)
        gate.wheelTick(at: 11.01 + EarlierLoadGate.wheelPause, rowOnScreen: true)
        XCTAssertNil(gate.pass(.moreFeed), "one page per spin")
    }
}
