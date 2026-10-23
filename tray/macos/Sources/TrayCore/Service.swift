import Foundation
import Network

/// The tray's connection to the component socket (core/components/rpc.js's
/// JSON lines): a hello as role `tray`, whose answer carries the tray's key
/// when the service can tell this is XenoNet's tray app. The connection
/// stays open: the service knows a tray runs while it does (Desktop hides
/// its own then), and its closing says the service went, and with it the
/// key.
public final class ComponentLink: @unchecked Sendable {
    private let connection: NWConnection
    private let queue = DispatchQueue(label: "dev.xenonet.tray.component")
    // Touched only on `queue`.
    private var buffer = Data()
    private var waiting: [CheckedContinuation<Void, Never>] = []
    private var ended = false

    public init(port: Int) {
        connection = NWConnection(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: UInt16(port)) ?? 1, using: .tcp)
    }

    /// Says hello with the descriptor's token, and answers the tray's key.
    /// Throws TrayError.refused with the service's reason when it doesn't
    /// take this tray, and any other error when it can't be reached.
    public func hello(token: String, timeout: TimeInterval = 5) async throws -> String {
        try await open(timeout: timeout)
        let hello: [String: Any] = ["id": 1, "method": "hello", "params": ["token": token, "role": "tray", "api": ["min": 1, "max": 1]]]
        var line = try JSONSerialization.data(withJSONObject: hello)
        line.append(10)
        try await send(line)
        let answer = try await readLine(timeout: timeout)
        guard let reply = try JSONSerialization.jsonObject(with: answer) as? [String: Any] else { throw TrayError.status(0, "XenoNet's answer wasn't JSON") }
        if let error = reply["error"] as? String { throw TrayError.refused(error) }
        guard let result = reply["result"] as? [String: Any], let key = result["trayKey"] as? String, key.utf8.count == 64 else {
            throw TrayError.refused("XenoNet answered without the tray's key")
        }
        watch()
        return key
    }

    /// Returns once the connection has ended.
    public func closed() async {
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            queue.async {
                if self.ended { c.resume() } else { self.waiting.append(c) }
            }
        }
    }

    public func close() {
        connection.cancel()
        queue.async { self.end() }
    }

    // Must run on `queue`.
    private func end() {
        guard !ended else { return }
        ended = true
        for c in waiting { c.resume() }
        waiting = []
    }

    /// Whether a continuation was resumed; touched only on `queue`.
    private final class Once: @unchecked Sendable {
        var done = false
    }

    private func open(timeout: TimeInterval) async throws {
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
            let once = Once()
            let finish: @Sendable (Error?) -> Void = { e in
                guard !once.done else { return }
                once.done = true
                if let e { c.resume(throwing: e) } else { c.resume() }
            }
            connection.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                switch state {
                case .ready:
                    finish(nil)
                case let .failed(e), let .waiting(e):
                    finish(e)
                    self.connection.cancel()
                case .cancelled:
                    finish(TrayError.status(0, "the connection closed"))
                    self.end()
                default:
                    break
                }
            }
            connection.start(queue: queue)
            queue.asyncAfter(deadline: .now() + timeout) { [weak self] in
                guard !once.done else { return }
                finish(TrayError.status(0, "XenoNet didn't answer"))
                self?.connection.cancel()
            }
        }
    }

    private func send(_ data: Data) async throws {
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
            connection.send(content: data, completion: .contentProcessed { e in
                if let e { c.resume(throwing: e) } else { c.resume() }
            })
        }
    }

    /// One line of the answer, without its newline.
    private func readLine(timeout: TimeInterval) async throws -> Data {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if let nl = buffer.firstIndex(of: 10) {
                let line = buffer[buffer.startIndex..<nl]
                buffer.removeSubrange(buffer.startIndex...nl)
                return Data(line)
            }
            if Date() > deadline || buffer.count > 1 << 20 { throw TrayError.status(0, "XenoNet didn't answer") }
            let chunk = try await receive()
            guard let chunk else { throw TrayError.status(0, "the connection closed") }
            buffer.append(chunk)
        }
    }

    private func receive() async throws -> Data? {
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Data?, Error>) in
            connection.receive(minimumIncompleteLength: 1, maximumLength: 64 << 10) { data, _, complete, error in
                if let error { c.resume(throwing: error) } else if let data, !data.isEmpty { c.resume(returning: data) } else if complete { c.resume(returning: nil) } else { c.resume(returning: Data()) }
            }
        }
    }

    /// After the hello: reads until the service closes the connection,
    /// which ends it here too. Nothing the service sends a tray matters yet.
    private func watch() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 64 << 10) { [weak self] data, _, complete, error in
            guard let self else { return }
            if error != nil || complete || data == nil {
                self.connection.cancel()
                self.end()
            } else {
                self.watch()
            }
        }
    }
}

/// The tray's routes on the service's port, with its key.
public struct TrayAPI: Sendable {
    public let port: Int
    let key: String
    let session: URLSession

    public init(port: Int, key: String) {
        self.port = port
        self.key = key
        let config = URLSessionConfiguration.ephemeral
        config.httpCookieStorage = nil
        config.urlCache = nil
        config.connectionProxyDictionary = [:]
        // The stream gets a ping every half minute or less.
        config.timeoutIntervalForRequest = 90
        session = URLSession(configuration: config)
    }

    func request(_ path: String, method: String = "GET", body: Data? = nil) -> URLRequest {
        var r = URLRequest(url: URL(string: "http://127.0.0.1:\(port)\(path)")!)
        r.httpMethod = method
        r.setValue(key, forHTTPHeaderField: "x-xenonet-tray")
        r.setValue("1", forHTTPHeaderField: "x-xenonet-api-version")
        if let body {
            r.httpBody = body
            r.setValue("application/json", forHTTPHeaderField: "content-type")
        }
        return r
    }

    static func check(_ response: URLResponse, _ data: Data) throws {
        guard let http = response as? HTTPURLResponse else { throw TrayError.status(0, "not an HTTP answer") }
        guard http.statusCode == 200 else {
            let said = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String
            if http.statusCode == 403 { throw TrayError.refused(said ?? "XenoNet refused the tray") }
            throw TrayError.status(http.statusCode, said ?? "XenoNet answered \(http.statusCode)")
        }
    }

    public func status() async throws -> TrayStatus {
        let (data, response) = try await session.data(for: request("/api/tray/v1/status"))
        try Self.check(response, data)
        return try TrayStatus.decode(data)
    }

    /// Asks the service to show Desktop's window, at `id` when it names
    /// a session or an actor.
    public func openConsole(id: String?) async throws -> OpenAnswer {
        let body = try JSONSerialization.data(withJSONObject: id.map { ["id": $0] } ?? [String: String]())
        let (data, response) = try await session.data(for: request("/api/tray/v1/open-console", method: "POST", body: body))
        try Self.check(response, data)
        return try JSONDecoder().decode(OpenAnswer.self, from: data)
    }

    /// Reads the event stream until it ends, handing each status and alert
    /// to `take`.
    public func events(_ take: @Sendable (StreamEvent) async -> Void) async throws {
        let (bytes, response) = try await session.bytes(for: request("/api/tray/v1/events"))
        try Self.check(response, Data())
        var parser = EventStreamParser()
        var chunk: [UInt8] = []
        chunk.reserveCapacity(4096)
        for try await b in bytes {
            chunk.append(b)
            // Lines are handed over whole; a byte at a time otherwise.
            guard b == 10 else { continue }
            for e in try parser.feed(chunk) { await take(e) }
            chunk.removeAll(keepingCapacity: true)
        }
    }
}
