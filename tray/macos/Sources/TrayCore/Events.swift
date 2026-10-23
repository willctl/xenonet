import Foundation

/// One event of a text/event-stream: its name and its data lines joined.
public struct StreamEvent: Equatable, Sendable {
    public var name: String
    public var data: String
}

/// Reads a text/event-stream as its bytes arrive: lines end in \n (a \r
/// before it is dropped), a blank line ends an event, a line starting with
/// a colon is a comment (the service's pings), and `retry:` and `id:` are
/// ignored. Bytes are kept until their line ends, so a chunk may end
/// anywhere.
public struct EventStreamParser: Sendable {
    private var line: [UInt8] = []
    private var name = "message"
    private var data: [String] = []
    /// A line longer than this ends the stream: the service's are far shorter.
    public static let maxLine = 1 << 20

    public init() {}

    public mutating func feed(_ bytes: some Sequence<UInt8>) throws -> [StreamEvent] {
        var out: [StreamEvent] = []
        for b in bytes {
            if b != 10 {
                line.append(b)
                if line.count > Self.maxLine { throw TrayError.status(0, "an event stream line was too long") }
                continue
            }
            if line.last == 13 { line.removeLast() }
            let text = String(decoding: line, as: UTF8.self)
            line.removeAll(keepingCapacity: true)
            if let e = take(text) { out.append(e) }
        }
        return out
    }

    private mutating func take(_ text: String) -> StreamEvent? {
        if text.isEmpty {
            defer {
                name = "message"
                data = []
            }
            return data.isEmpty ? nil : StreamEvent(name: name, data: data.joined(separator: "\n"))
        }
        if text.hasPrefix(":") { return nil }
        let field: Substring
        var value: Substring
        if let colon = text.firstIndex(of: ":") {
            field = text[..<colon]
            value = text[text.index(after: colon)...]
            if value.hasPrefix(" ") { value = value.dropFirst() }
        } else {
            field = Substring(text)
            value = ""
        }
        switch field {
        case "event": name = String(value)
        case "data": data.append(String(value))
        default: break
        }
        return nil
    }
}
