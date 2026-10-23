import Foundation
import XCTest
@testable import TrayCore

/// The fixtures the service's JS and Rust tests check against the tray's
/// schemas (test/fixtures/tray), so the app reads what the service sends.
private func fixture(_ name: String) throws -> Data {
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("../../../../test/fixtures/tray").standardized
    return try Data(contentsOf: root.appendingPathComponent(name))
}

final class FixtureTests: XCTestCase {
    func testTheStatusFixtureDecodes() throws {
        let s = try TrayStatus.decode(fixture("status.json"))
        XCTAssertEqual(s.version, "0.4.1")
        XCTAssertEqual(s.instance, "installed")
        XCTAssertEqual(s.needsYou.count, 4)
        XCTAssertEqual(s.needsYou.items.map(\.kind), ["actor", "question", "permission", "requests"])
        XCTAssertEqual(s.needsYou.items[2].host, "studio")
        XCTAssertEqual(s.needsYou.items[2].stale, true)
        XCTAssertEqual(s.needsYou.items[3].count, 2)
        XCTAssertNil(s.needsYou.items[3].id)
        XCTAssertEqual(s.gateway, .init(on: true, state: "online", url: "https://xenonet.example.ts.net/mcp", problem: nil))
        XCTAssertEqual(s.network, .init(name: "home", host: "studio", link: "live", hosts: 2, online: 1))
    }

    func testTheEventFixtureDecodes() throws {
        let e = try TrayEvent.decode(fixture("event.json"))
        XCTAssertEqual(e.kind, "needs-you")
        XCTAssertEqual(e.item.words, "asks")
        XCTAssertEqual(e.item.text, "Should the limit be per key or per account?")
    }

    func testAnotherSchemaIsRefused() throws {
        XCTAssertThrowsError(try TrayStatus.decode(fixture("event.json")))
        var other = try JSONSerialization.jsonObject(with: fixture("status.json")) as! [String: Any]
        other["schema"] = "tray/status/v2"
        XCTAssertThrowsError(try TrayStatus.decode(JSONSerialization.data(withJSONObject: other))) { error in
            XCTAssertEqual(error as? TrayError, .schema("tray/status/v2"))
        }
    }

    func testFieldsAddedLaterAreIgnored() throws {
        var more = try JSONSerialization.jsonObject(with: fixture("status.json")) as! [String: Any]
        more["somethingNew"] = ["a": 1]
        XCTAssertEqual(try TrayStatus.decode(JSONSerialization.data(withJSONObject: more)).version, "0.4.1")
    }
}

final class MenuTests: XCTestCase {
    func testTheMenuListsWhatNeedsYouInTheConsolesWords() throws {
        let s = try TrayStatus.decode(fixture("status.json"))
        XCTAssertEqual(TrayMenu.icon(.live(s)), .needsYou(4))
        XCTAssertEqual(TrayMenu.entries(.live(s)), [
            .info("XenoNet 0.4.1 is running"),
            .separator,
            .info("Needs you (4)"),
            .action("Wren needs permission", .openConsole(id: "actor:wren")),
            .action("Rate-limit the API asks", .openConsole(id: "claude:0190d3e0-0000-7000-8000-000000000001")),
            .action("Move the docs needs permission · on studio (may be over)", .openConsole(id: "@studio:codex:019a0000-0000-7000-8000-000000000002")),
            .action("2 programs ask to join XenoNet", .openConsole(id: nil)),
            .separator,
            .action("Open console", .openConsole(id: nil)),
            .separator,
            .info("Gateway: online"),
            .info("Network home: connected · 1 of 2 other machines online"),
            .separator,
            .action("Quit the tray", .quit),
        ])
    }

    func testNothingWaitingAndTheGatewaysOtherStates() throws {
        var s = try TrayStatus.decode(fixture("status.json"))
        s.needsYou = .init(count: 0, items: [])
        s.gateway = .init(on: true, state: "problem", url: nil, problem: "port 48850 is in use by another program")
        s.network = .init(name: nil, host: nil, link: "off", hosts: 0, online: 0)
        XCTAssertEqual(TrayMenu.icon(.live(s)), .running)
        let entries = TrayMenu.entries(.live(s))
        XCTAssertTrue(entries.contains(.info("Nothing needs you")))
        XCTAssertTrue(entries.contains(.info("Gateway: port 48850 is in use by another program")))
        XCTAssertFalse(entries.contains { if case let .info(t) = $0 { return t.hasPrefix("Network") } else { return false } })
        s.gateway = .init(on: false, state: "off", url: nil, problem: nil)
        XCTAssertTrue(TrayMenu.entries(.live(s)).contains(.info("Gateway: off")))
    }

    func testStoppedAndRefused() {
        XCTAssertEqual(TrayMenu.icon(.stopped), .stopped)
        XCTAssertEqual(TrayMenu.entries(.stopped), [.info("XenoNet isn't running"), .separator, .action("Quit the tray", .quit)])
        XCTAssertEqual(TrayMenu.icon(.refused("x")), .stopped)
        let why = "XenoNet didn't take this as its tray: only XenoNet Tray.app's own executable, in /Applications or your own Applications folder, can be XenoNet's tray"
        XCTAssertEqual(TrayMenu.entries(.refused(why)), [
            .info("XenoNet didn't take this tray:"),
            .info("only XenoNet Tray.app's own executable, in /Applications or"),
            .info("your own Applications folder, can be XenoNet's tray"),
            .separator,
            .action("Quit the tray", .quit),
        ])
    }

    func testTheMenuAsTextAndAnEntryByItsTitle() throws {
        let s = try TrayStatus.decode(fixture("status.json"))
        let entries = TrayMenu.entries(.live(s))
        let text = TrayMenu.describe(TrayMenu.icon(.live(s)), entries)
        XCTAssertTrue(text.hasPrefix("icon: needs you (4)\n[XenoNet 0.4.1 is running]\n-\n[Needs you (4)]\nWren needs permission\n"), text)
        XCTAssertEqual(TrayMenu.action(titled: "Rate-limit the API asks", in: entries), .openConsole(id: "claude:0190d3e0-0000-7000-8000-000000000001"))
        XCTAssertEqual(TrayMenu.action(titled: "Open console", in: entries), .openConsole(id: nil))
        XCTAssertNil(TrayMenu.action(titled: "Needs you (4)", in: entries), "a line that does nothing")
    }

    func testALongQueueListsTheFirstAndSaysHowManyMore() throws {
        var s = try TrayStatus.decode(fixture("status.json"))
        let item = s.needsYou.items[1]
        s.needsYou = .init(count: 20, items: Array(repeating: item, count: 20))
        let actions = TrayMenu.entries(.live(s)).filter { if case .action = $0 { return true } else { return false } }
        XCTAssertEqual(actions.count, TrayMenu.maxItems + 3)
        XCTAssertTrue(actions.contains(.action("8 more in the console", .openConsole(id: nil))))
    }

    func testANotificationSaysWhatItWaitsForAndAsks() throws {
        let e = try TrayEvent.decode(fixture("event.json"))
        let n = TrayMenu.notification(e.item)
        XCTAssertEqual(n.title, "Rate-limit the API")
        XCTAssertEqual(n.body, "Asks: Should the limit be per key or per account?")
    }
}

final class LocateTests: XCTestCase {
    func testThePortAndTheConnectionFile() {
        XCTAssertEqual(Locate.port(arguments: [], environment: [:]), 47840)
        XCTAssertEqual(Locate.port(arguments: [], environment: ["XENONET_PORT": "48321"]), 48321)
        XCTAssertEqual(Locate.port(arguments: ["tray", "--port", "48322"], environment: ["XENONET_PORT": "48321"]), 48322)
        XCTAssertEqual(Locate.port(arguments: ["tray", "--port=48323"], environment: [:]), 48323)
        XCTAssertEqual(Locate.port(arguments: ["tray", "--port", "nope"], environment: ["XENONET_PORT": "0"]), 47840)
        XCTAssertEqual(Locate.descriptorURL(port: 47840, environment: [:], home: "/Users/me").path, "/Users/me/Library/Application Support/XenoNet/runtime/service-47840.json")
        XCTAssertEqual(Locate.descriptorURL(port: 1, environment: ["XDG_STATE_HOME": "/s"], home: "/Users/me").path, "/s/xenonet/runtime/service-1.json")
        XCTAssertEqual(Locate.descriptorURL(port: 1, environment: ["XENONET_RUNTIME_DIR": "/r", "XDG_STATE_HOME": "/s"], home: "/Users/me").path, "/r/service-1.json")
    }

    func testOnlyADescriptorTheServiceWroteIsRead() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("xenonet-tray-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = dir.appendingPathComponent("service-1.json")
        try Data(#"{"port":50123,"token":"\#(String(repeating: "a1", count: 32))","pid":7,"version":"0.4.1"}"#.utf8).write(to: file)
        XCTAssertEqual(try Descriptor.read(file).port, 50123)
        try Data(#"{"port":50123,"token":"short"}"#.utf8).write(to: file)
        XCTAssertThrowsError(try Descriptor.read(file))
    }
}

final class EventStreamTests: XCTestCase {
    func testEventsComeWholeHoweverTheBytesArrive() throws {
        let text = "retry: 3000\n\n: ping\n\nevent: status\ndata: {\"a\":1}\n\nevent: alert\r\ndata: {\"b\":\r\ndata: 2}\r\n\r\n"
        var p = EventStreamParser()
        var got: [StreamEvent] = []
        for b in Array(text.utf8) { got += try p.feed([b]) }
        XCTAssertEqual(got, [StreamEvent(name: "status", data: "{\"a\":1}"), StreamEvent(name: "alert", data: "{\"b\":\n2}")])
        var q = EventStreamParser()
        XCTAssertEqual(try q.feed(Array(text.utf8)), got)
    }

    func testALineTooLongEndsTheStream() {
        var p = EventStreamParser()
        XCTAssertThrowsError(try p.feed(Array(repeating: UInt8(97), count: EventStreamParser.maxLine + 1)))
    }
}
