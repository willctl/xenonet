import AppKit
import TrayCore

/// XENONET_TRAY_SNAPSHOT names a folder for checking the tray without
/// reading the screen (docs/guide/development.md): the tray writes what it
/// shows there, menu.txt (TrayMenu.describe) and icon.png, each time it
/// changes, and takes a file called `choose`, holding a menu item's title,
/// as a click on that item. Nothing is written without it. What it shows is
/// what /api/summary says to any program, and the tray's key never goes
/// there.
@MainActor
final class Snapshot {
    private let dir: URL
    private var timer: Timer?

    init?(environment: [String: String]) {
        guard let path = environment["XENONET_TRAY_SNAPSHOT"], !path.isEmpty else { return nil }
        dir = URL(fileURLWithPath: path)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    func write(_ icon: IconState, _ entries: [MenuEntry], button: NSStatusBarButton?) {
        try? Data(TrayMenu.describe(icon, entries).utf8).write(to: dir.appendingPathComponent("menu.txt"), options: .atomic)
        guard let button, let rep = button.bitmapImageRepForCachingDisplay(in: button.bounds) else { return }
        button.cacheDisplay(in: button.bounds, to: rep)
        if let png = rep.representation(using: .png, properties: [:]) {
            try? png.write(to: dir.appendingPathComponent("icon.png"), options: .atomic)
        }
    }

    /// Looks for `choose` every half second, and hands its title over.
    func watch(_ choose: @escaping @MainActor (String) -> Void) {
        let file = dir.appendingPathComponent("choose")
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { _ in
            MainActor.assumeIsolated {
                guard let title = try? String(contentsOf: file, encoding: .utf8) else { return }
                try? FileManager.default.removeItem(at: file)
                choose(title.trimmingCharacters(in: .whitespacesAndNewlines))
            }
        }
    }
}
