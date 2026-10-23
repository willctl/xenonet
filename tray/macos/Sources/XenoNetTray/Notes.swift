import Foundation
import TrayCore
import UserNotifications

/// System notifications for what starts waiting on you, one per item: a
/// newer one replaces it, it's withdrawn once the item stops waiting, and
/// a click opens the console at it. macOS asks you once whether the tray
/// may post them.
@MainActor
final class Notes: NSObject, UNUserNotificationCenterDelegate {
    private var center: UNUserNotificationCenter?
    private var onOpen: ((String?) -> Void)?

    func start(onOpen: @escaping (String?) -> Void) {
        // Only an app bundle has a notification center: a bare build of
        // the executable (swift run) has none, and asking would crash it.
        guard Bundle.main.bundleIdentifier != nil, Bundle.main.bundleURL.pathExtension == "app" else {
            log.info("not in an app bundle, so no notifications")
            return
        }
        let c = UNUserNotificationCenter.current()
        c.delegate = self
        center = c
        self.onOpen = onOpen
        c.requestAuthorization(options: [.alert, .sound]) { granted, error in
            if let error {
                log.error("notifications: \(error.localizedDescription, privacy: .public)")
            } else {
                log.info("notifications allowed: \(granted, privacy: .public)")
            }
        }
    }

    func alert(_ e: TrayEvent) {
        guard let center else { return }
        switch e.kind {
        case "needs-you":
            let (title, body) = TrayMenu.notification(e.item)
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            content.sound = .default
            content.threadIdentifier = "needs-you"
            content.userInfo = ["id": e.item.id ?? ""]
            center.add(UNNotificationRequest(identifier: e.item.key, content: content, trigger: nil)) { error in
                if let error { log.error("a notification wasn't posted: \(error.localizedDescription, privacy: .public)") }
            }
        case "resolved":
            center.removeDeliveredNotifications(withIdentifiers: [e.item.key])
        default:
            break
        }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let id = response.notification.request.content.userInfo["id"] as? String
        await MainActor.run { self.onOpen?(id?.isEmpty == false ? id : nil) }
    }

    // Shown even while the tray is the active app, which it rarely is.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }
}
