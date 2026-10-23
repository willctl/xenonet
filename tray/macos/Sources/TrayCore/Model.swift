import Foundation

// The tray API's types, written to match its schemas in schemas/tray/
// (core/schema/tray.js). The tests decode the fixtures the service's own
// tests check against those schemas (test/fixtures/tray), so the three
// agree. A field the schema adds later is ignored here until the app
// reads it.

/// tray/status/v1: what GET /api/tray/v1/status answers, and what the
/// event stream sends as `status` when it changes.
public struct TrayStatus: Codable, Equatable, Sendable {
    public static let schemaId = "tray/status/v1"

    public var schema: String
    public var running: Bool
    public var version: String
    public var instance: String?
    public var url: String
    public var desktop: Bool
    public var working: Int
    public var needsYou: NeedsYou
    public var gateway: Gateway
    public var network: Network
    public var at: Double

    public struct NeedsYou: Codable, Equatable, Sendable {
        public var count: Int
        public var items: [TrayItem]
    }

    /// The actor gateway: state is off, starting, online or problem.
    public struct Gateway: Codable, Equatable, Sendable {
        public var on: Bool
        public var state: String
        public var url: String?
        public var problem: String?
    }

    /// This machine's place in its network; link is off when it's in none.
    public struct Network: Codable, Equatable, Sendable {
        public var name: String?
        public var host: String?
        public var link: String
        public var hosts: Int
        public var online: Int
    }

    /// A status from the API's JSON, refused when it says it's another schema.
    public static func decode(_ data: Data) throws -> TrayStatus {
        let s = try JSONDecoder().decode(TrayStatus.self, from: data)
        guard s.schema == schemaId else { throw TrayError.schema(s.schema) }
        return s
    }
}

/// An item of the Needs you queue: its title, and the words the console
/// says after it ("asks", "needs permission"…).
public struct TrayItem: Codable, Equatable, Sendable {
    public var key: String
    public var kind: String
    public var id: String?
    public var title: String
    public var words: String
    public var app: String?
    public var host: String?
    public var text: String?
    public var since: Double?
    public var stale: Bool?
    public var count: Int?

    public init(key: String, kind: String, id: String?, title: String, words: String, app: String? = nil, host: String? = nil, text: String? = nil, since: Double? = nil, stale: Bool? = nil, count: Int? = nil) {
        self.key = key
        self.kind = kind
        self.id = id
        self.title = title
        self.words = words
        self.app = app
        self.host = host
        self.text = text
        self.since = since
        self.stale = stale
        self.count = count
    }
}

/// tray/event/v1: an item that started (needs-you) or stopped (resolved)
/// waiting on you, sent as `alert`.
public struct TrayEvent: Codable, Equatable, Sendable {
    public static let schemaId = "tray/event/v1"

    public var schema: String
    public var kind: String
    public var item: TrayItem
    public var at: Double

    public static func decode(_ data: Data) throws -> TrayEvent {
        let e = try JSONDecoder().decode(TrayEvent.self, from: data)
        guard e.schema == schemaId else { throw TrayError.schema(e.schema) }
        return e
    }
}

/// What POST /api/tray/v1/open-console answers: opened is "desktop" when
/// a connected Desktop showed its window, null when none is connected.
public struct OpenAnswer: Codable, Equatable, Sendable {
    public var ok: Bool
    public var opened: String?
    public var url: String
}

public enum TrayError: Error, Equatable, Sendable {
    case schema(String)
    /// The service didn't take this tray, and said why.
    case refused(String)
    /// The service answered with an HTTP status the tray can't use.
    case status(Int, String)
}
