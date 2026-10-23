// XenoNet Tray: XenoNet's menu bar item on macOS. It finds the service on
// its port (--port or XENONET_PORT, 47840 by default), gets the tray's key
// from the component socket, and follows the tray API's event stream:
// whether XenoNet runs, what needs you, the gateway's state. Its menu
// opens the console, and new items that wait on you post a notification.
import AppKit
import TrayCore

@MainActor
enum Main {
    static func main() {
        let app = NSApplication.shared
        // A menu bar item only: no Dock icon, no main menu (LSUIElement too).
        app.setActivationPolicy(.accessory)
        let delegate = TrayApp()
        app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}

MainActor.assumeIsolated { Main.main() }
