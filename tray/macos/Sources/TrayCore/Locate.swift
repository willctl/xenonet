import Foundation

/// Where the tray finds the service, as the Tools find it
/// (core/components/runtime.js): the port from --port or XENONET_PORT
/// (47840 by default), and the component socket's descriptor,
/// `service-<port>.json` in the runtime folder, which holds the socket's
/// port and a token new at each start of the service.
public enum Locate {
    public static let defaultPort = 47840
    /// Desktop's bundle id: the tray opens it for the installed XenoNet.
    public static let desktopBundleId = "dev.xenonet.app"

    /// The service's port: `--port <n>` among the arguments, else
    /// XENONET_PORT, else 47840.
    public static func port(arguments: [String], environment: [String: String]) -> Int {
        if let i = arguments.firstIndex(of: "--port"), i + 1 < arguments.count, let p = valid(arguments[i + 1]) { return p }
        if let given = arguments.first(where: { $0.hasPrefix("--port=") }), let p = valid(String(given.dropFirst("--port=".count))) { return p }
        if let p = environment["XENONET_PORT"].flatMap(valid) { return p }
        return defaultPort
    }

    private static func valid(_ s: String) -> Int? {
        guard let n = Int(s), (1...65535).contains(n) else { return nil }
        return n
    }

    /// The runtime folder: XENONET_RUNTIME_DIR, else under XDG_STATE_HOME
    /// when it's set, else ~/Library/Application Support/XenoNet/runtime.
    public static func runtimeDirectory(environment: [String: String], home: String) -> URL {
        if let dir = environment["XENONET_RUNTIME_DIR"], !dir.isEmpty { return URL(fileURLWithPath: dir) }
        if let xdg = environment["XDG_STATE_HOME"], !xdg.isEmpty {
            return URL(fileURLWithPath: xdg).appendingPathComponent("xenonet").appendingPathComponent("runtime")
        }
        return URL(fileURLWithPath: home).appendingPathComponent("Library/Application Support/XenoNet/runtime")
    }

    public static func descriptorURL(port: Int, environment: [String: String], home: String) -> URL {
        runtimeDirectory(environment: environment, home: home).appendingPathComponent("service-\(port).json")
    }
}

/// The component socket's descriptor.
public struct Descriptor: Codable, Equatable, Sendable {
    public var port: Int
    public var token: String
    public var pid: Int?
    public var version: String?

    /// The descriptor, refused unless its port and token have the shape
    /// the service writes.
    public static func read(_ url: URL) throws -> Descriptor {
        let d = try JSONDecoder().decode(Descriptor.self, from: Data(contentsOf: url))
        let hex = d.token.utf8.count == 64 && d.token.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
        guard (1...65535).contains(d.port), hex else { throw TrayError.refused("XenoNet's connection file isn't one the service wrote") }
        return d
    }
}
