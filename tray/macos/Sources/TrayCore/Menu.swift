import Foundation

/// How the tray stands with the service.
public enum Standing: Equatable, Sendable {
    /// Looking for the service.
    case connecting
    /// No service answered on this port.
    case stopped
    /// The service answered and didn't take this tray, saying why: one not
    /// installed in an Applications folder, or started from under an agent.
    case refused(String)
    /// Connected, with the service's latest status.
    case live(TrayStatus)
}

/// The menu bar icon's state.
public enum IconState: Equatable, Sendable {
    case running
    /// Something waits on you: how many.
    case needsYou(Int)
    case stopped
}

public enum MenuAction: Equatable, Sendable {
    /// The console, at a session or an actor when it names one.
    case openConsole(id: String?)
    case quit
}

public enum MenuEntry: Equatable, Sendable {
    /// A line that says something and does nothing.
    case info(String)
    case action(String, MenuAction)
    case separator
}

/// What the tray shows for a standing: its icon and its menu, in the
/// console's words. AppKit draws them (Sources/XenoNetTray).
public enum TrayMenu {
    /// The most items of the Needs you queue the menu lists; the queue's
    /// count says how many there are in all.
    public static let maxItems = 12

    public static func icon(_ s: Standing) -> IconState {
        guard case let .live(status) = s else { return .stopped }
        return status.needsYou.count > 0 ? .needsYou(status.needsYou.count) : .running
    }

    /// An item as the console's Needs you panel says it: its title, then
    /// what it waits for, and the machine it's on when that isn't this one.
    public static func label(_ item: TrayItem) -> String {
        var s = "\(item.title) \(item.words)"
        if let host = item.host { s += " · on \(host)" }
        if item.stale == true { s += " (may be over)" }
        return clip(s, 80)
    }

    public static func gatewayLine(_ g: TrayStatus.Gateway) -> String {
        switch g.state {
        case "off": return "Gateway: off"
        case "starting": return "Gateway: starting"
        case "online": return "Gateway: online"
        case "problem": return "Gateway: \(clip(g.problem ?? "something's wrong", 70))"
        default: return "Gateway: \(g.state)"
        }
    }

    /// This machine's link to its network, or nil when it's in none.
    public static func networkLine(_ n: TrayStatus.Network) -> String? {
        guard n.link != "off", let name = n.name else { return nil }
        let link: String
        switch n.link {
        case "live": link = "connected"
        case "connecting": link = "connecting"
        case "offline": link = "offline"
        case "refused": link = "refused by its gateway"
        case "removed": link = "removed from it"
        default: link = n.link
        }
        let others = n.hosts == 0 ? "" : " · \(n.online) of \(n.hosts) other \(n.hosts == 1 ? "machine" : "machines") online"
        return clip("Network \(name): \(link)\(others)", 80)
    }

    public static func entries(_ s: Standing) -> [MenuEntry] {
        var out: [MenuEntry] = []
        switch s {
        case .connecting:
            out.append(.info("Connecting to XenoNet…"))
        case .stopped:
            out.append(.info("XenoNet isn't running"))
        case let .refused(why):
            out.append(.info("XenoNet didn't take this tray:"))
            // The service's answer starts by saying what the header says.
            let reason = why.replacingOccurrences(of: "XenoNet didn't take this as its tray: ", with: "")
            for line in wrap(reason, 60).prefix(4) { out.append(.info(line)) }
        case let .live(status):
            out.append(.info("XenoNet \(status.version) is running"))
            out.append(.separator)
            let items = status.needsYou.items.prefix(maxItems)
            if status.needsYou.count == 0 && items.isEmpty {
                out.append(.info("Nothing needs you"))
            } else {
                out.append(.info("Needs you (\(status.needsYou.count))"))
                for item in items {
                    out.append(.action(label(item), .openConsole(id: item.id)))
                }
                let more = status.needsYou.items.count - items.count
                if more > 0 { out.append(.action("\(more) more in the console", .openConsole(id: nil))) }
            }
            out.append(.separator)
            out.append(.action("Open console", .openConsole(id: nil)))
            out.append(.separator)
            out.append(.info(gatewayLine(status.gateway)))
            if let line = networkLine(status.network) { out.append(.info(line)) }
        }
        out.append(.separator)
        out.append(.action("Quit the tray", .quit))
        return out
    }

    /// A notification for an item that started waiting: its title, and
    /// what it waits for with the question when there is one.
    public static func notification(_ item: TrayItem) -> (title: String, body: String) {
        let what = [item.words.prefix(1).uppercased() + item.words.dropFirst(), item.text].compactMap { $0 }.filter { !$0.isEmpty }
        let host = item.host.map { " · on \($0)" } ?? ""
        return (clip(item.title + host, 120), clip(what.joined(separator: ": "), 240))
    }

    /// The menu as text, one entry a line: what a check by hand or a test
    /// reads (XENONET_TRAY_SNAPSHOT). A line that does nothing is in
    /// brackets, a separator is a dash.
    public static func describe(_ icon: IconState, _ entries: [MenuEntry]) -> String {
        let head: String
        switch icon {
        case .running: head = "icon: running"
        case let .needsYou(n): head = "icon: needs you (\(n))"
        case .stopped: head = "icon: stopped"
        }
        let lines = entries.map { e -> String in
            switch e {
            case let .info(t): return "[\(t)]"
            case let .action(t, _): return t
            case .separator: return "-"
            }
        }
        return ([head] + lines).joined(separator: "\n") + "\n"
    }

    /// The action of the entry titled `title`, as a click on it.
    public static func action(titled title: String, in entries: [MenuEntry]) -> MenuAction? {
        for e in entries {
            if case let .action(t, a) = e, t == title { return a }
        }
        return nil
    }

    /// Words in lines of at most `width` characters, since a menu item's
    /// title doesn't wrap.
    static func wrap(_ s: String, _ width: Int) -> [String] {
        var lines: [String] = []
        var line = ""
        for word in s.split(separator: " ") {
            if !line.isEmpty, line.count + 1 + word.count > width {
                lines.append(line)
                line = ""
            }
            line += line.isEmpty ? String(word) : " \(word)"
        }
        if !line.isEmpty { lines.append(line) }
        return lines
    }

    static func clip(_ s: String, _ max: Int) -> String {
        s.count <= max ? s : String(s.prefix(max - 1)) + "…"
    }
}
