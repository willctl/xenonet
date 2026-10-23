import AppKit
import TrayCore
import os

let log = Logger(subsystem: "dev.xenonet.tray", category: "tray")

/// The menu bar item and the connection behind it.
@MainActor
final class TrayApp: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private let port: Int
    private let descriptor: URL
    private var item: NSStatusItem?
    private var standing: Standing = .connecting
    private var shown: [MenuEntry] = []
    private var menuOpen = false
    private var pending: [MenuEntry]?
    private var api: TrayAPI?
    private var runner: Task<Void, Never>?
    private let notes = Notes()
    private let snapshot: Snapshot?

    override init() {
        let env = ProcessInfo.processInfo.environment
        port = Locate.port(arguments: CommandLine.arguments, environment: env)
        descriptor = Locate.descriptorURL(port: port, environment: env, home: NSHomeDirectory())
        snapshot = Snapshot(environment: env)
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        item.button?.imagePosition = .imageLeading
        self.item = item
        show(.connecting)
        notes.start { [weak self] id in self?.open(id) }
        snapshot?.watch { [weak self] title in
            guard let self, let action = TrayMenu.action(titled: title, in: self.shown) else { return }
            self.perform(action)
        }
        runner = Task { [weak self] in await self?.run() }
    }

    func applicationWillTerminate(_ notification: Notification) {
        runner?.cancel()
    }

    // MARK: the connection

    /// Connects, follows the service until it goes, and tries again: soon
    /// after a connection that worked, then less often, at most every 15 s.
    private func run() async {
        var wait = 1.0
        while !Task.isCancelled {
            wait = await session() ? 1 : min(wait * 2, 15)
            try? await Task.sleep(nanoseconds: UInt64(wait * 1e9))
        }
    }

    /// One connection to the service, until it ends. True when it got as
    /// far as a status.
    private func session() async -> Bool {
        guard let d = try? Descriptor.read(descriptor) else {
            show(.stopped)
            return false
        }
        let link = ComponentLink(port: d.port)
        let api: TrayAPI
        do {
            api = TrayAPI(port: port, key: try await link.hello(token: d.token))
            show(.live(try await api.status()))
        } catch let TrayError.refused(why) {
            log.error("XenoNet didn't take this tray: \(why, privacy: .public)")
            link.close()
            show(.refused(why))
            return false
        } catch {
            link.close()
            show(.stopped)
            return false
        }
        self.api = api
        // The app lives as long as its process, so this is never left behind.
        let me = self
        await withTaskGroup(of: Void.self) { group in
            group.addTask { await link.closed() }
            group.addTask {
                do {
                    try await api.events { e in await me.take(e) }
                } catch {
                    log.info("the event stream ended: \(error.localizedDescription, privacy: .public)")
                }
            }
            await group.next()
            // Either ending ends both.
            link.close()
            group.cancelAll()
        }
        self.api = nil
        show(.connecting)
        return true
    }

    private func take(_ e: StreamEvent) {
        switch e.name {
        case "status":
            if let s = try? TrayStatus.decode(Data(e.data.utf8)) { show(.live(s)) }
        case "alert":
            if let a = try? TrayEvent.decode(Data(e.data.utf8)) { notes.alert(a) }
        default:
            break
        }
    }

    // MARK: what it shows

    private func show(_ s: Standing) {
        standing = s
        let icon = TrayMenu.icon(s)
        draw(icon)
        let entries = TrayMenu.entries(s)
        guard entries != shown else { return }
        // An open menu isn't swapped out from under the pointer.
        if menuOpen {
            pending = entries
            return
        }
        rebuild(entries)
        snapshot?.write(icon, entries, button: item?.button)
    }

    private func draw(_ state: IconState) {
        guard let button = item?.button else { return }
        switch state {
        case .running:
            button.image = Glyph.ring(lit: true)
            button.title = ""
            button.appearsDisabled = false
            button.toolTip = "XenoNet is running"
        case let .needsYou(n):
            button.image = Glyph.waiting()
            button.title = " \(min(n, 99))"
            button.appearsDisabled = false
            button.toolTip = n == 1 ? "1 thing needs you" : "\(n) things need you"
        case .stopped:
            button.image = Glyph.ring(lit: false)
            button.title = ""
            button.appearsDisabled = true
            button.toolTip = "XenoNet isn't running"
        }
        button.setAccessibilityLabel("XenoNet: \(button.toolTip ?? "")")
    }

    private func rebuild(_ entries: [MenuEntry]) {
        shown = entries
        let menu = NSMenu()
        menu.autoenablesItems = false
        menu.delegate = self
        for e in entries {
            switch e {
            case .separator:
                menu.addItem(.separator())
            case let .info(text):
                let i = NSMenuItem(title: text, action: nil, keyEquivalent: "")
                i.isEnabled = false
                menu.addItem(i)
            case let .action(text, action):
                let i = NSMenuItem(title: text, action: #selector(choose(_:)), keyEquivalent: action == .quit ? "q" : "")
                i.target = self
                i.representedObject = action
                menu.addItem(i)
            }
        }
        item?.menu = menu
    }

    func menuWillOpen(_ menu: NSMenu) {
        menuOpen = true
    }

    func menuDidClose(_ menu: NSMenu) {
        menuOpen = false
        if let p = pending {
            pending = nil
            rebuild(p)
            snapshot?.write(TrayMenu.icon(standing), p, button: item?.button)
        }
    }

    @objc private func choose(_ sender: NSMenuItem) {
        guard let action = sender.representedObject as? MenuAction else { return }
        perform(action)
    }

    private func perform(_ action: MenuAction) {
        switch action {
        case .quit:
            NSApp.terminate(nil)
        case let .openConsole(id):
            open(id)
        }
    }

    // MARK: opening the console

    /// The console, at `id` when it names a session or an actor: Desktop's
    /// window when a Desktop is connected to the service; else Desktop
    /// itself for the installed XenoNet, when it's installed; else the
    /// console in the browser.
    func open(_ id: String?) {
        Task {
            var url = "http://127.0.0.1:\(port)/"
            var instance: String?
            if case let .live(s) = standing {
                url = s.url
                instance = s.instance
            }
            if let api {
                do {
                    let answer = try await api.openConsole(id: id)
                    if answer.opened == "desktop" {
                        log.info("open console: Desktop showed its window")
                        return
                    }
                    url = answer.url
                } catch {
                    log.info("open-console: \(error.localizedDescription, privacy: .public)")
                }
            }
            // Desktop is the installed XenoNet's console. Another instance
            // (a demo, XenoNet Dev on its own port) shows in the browser.
            if port == Locate.defaultPort, instance == nil || instance == "installed",
               let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: Locate.desktopBundleId) {
                do {
                    _ = try await NSWorkspace.shared.openApplication(at: app, configuration: NSWorkspace.OpenConfiguration())
                    log.info("open console: opened Desktop")
                } catch {
                    log.error("Desktop didn't open: \(error.localizedDescription, privacy: .public)")
                }
                return
            }
            // Only this machine's console, whatever the answer said.
            if let u = URL(string: url), u.scheme == "http", u.host == "127.0.0.1", u.port == port {
                let opened = NSWorkspace.shared.open(u)
                log.info("open console: the browser at \(u.absoluteString, privacy: .public) (\(opened ? "opened" : "didn't open", privacy: .public))")
            }
        }
    }
}

/// The menu bar glyphs: a ring that follows the menu bar's colors, with a
/// dot while XenoNet runs and none while it's stopped, and an amber disc
/// while something needs you, as Desktop's tray turns amber.
enum Glyph {
    static func ring(lit: Bool) -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { rect in
            NSColor.black.set()
            let ring = NSBezierPath(ovalIn: rect.insetBy(dx: 3, dy: 3))
            ring.lineWidth = 1.6
            ring.stroke()
            if lit { NSBezierPath(ovalIn: NSRect(x: 6.5, y: 6.5, width: 5, height: 5)).fill() }
            return true
        }
        image.isTemplate = true
        return image
    }

    static func waiting() -> NSImage {
        NSImage(size: NSSize(width: 18, height: 18), flipped: false) { rect in
            NSColor(srgbRed: 0.96, green: 0.65, blue: 0.14, alpha: 1).setFill()
            NSBezierPath(ovalIn: rect.insetBy(dx: 2.5, dy: 2.5)).fill()
            return true
        }
    }
}
