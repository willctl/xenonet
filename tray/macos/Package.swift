// swift-tools-version:6.0
// XenoNet's tray app for macOS (docs/design/always-on.md, "The tray app"):
// a menu bar item that shows whether XenoNet runs, what needs you and the
// gateway's state, and opens the console. scripts/tray-build.mjs builds it
// for both Mac CPUs and makes XenoNet Tray.app around it.
import PackageDescription

let package = Package(
    name: "XenoNetTray",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "XenoNetTray", targets: ["XenoNetTray"]),
    ],
    targets: [
        // What the app knows without AppKit: the API's types, where the
        // service is, the menu it shows, and the event stream's format.
        .target(name: "TrayCore"),
        .executableTarget(name: "XenoNetTray", dependencies: ["TrayCore"]),
        .testTarget(name: "TrayCoreTests", dependencies: ["TrayCore"]),
    ]
)
