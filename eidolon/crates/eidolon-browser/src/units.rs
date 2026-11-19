//! The systemd units and the profile's `user.js`, kept as files in
//! `units/` and rendered with the ports filled in.
//!
//! Nothing of Eidolon's runs in the guest: these units start stock
//! programs (Firefox, Xvfb, x11vnc) and systemd keeps them alive.

/// The guest user that owns the profile and runs the browser.
pub const USER: &str = "eidolon";
/// The Firefox profile folder in the guest.
pub const PROFILE_PATH: &str = "/home/eidolon/profile";
/// Where the browser saves downloads.
pub const DOWNLOADS_PATH: &str = "/home/eidolon/files/downloads";
/// Where files for uploads are put.
pub const UPLOADS_PATH: &str = "/home/eidolon/files/uploads";
/// The folder for the sign-in window's password file (a tmpfs).
pub const RUN_PATH: &str = "/run/eidolon";
/// The sign-in window's password file.
pub const VNC_PASSWORD_PATH: &str = "/run/eidolon/vnc.pass";
/// The default guest port of the automation socket.
pub const DEFAULT_AUTOMATION_PORT: u16 = 9222;
/// The default guest port of the sign-in window's screen share.
pub const DEFAULT_VNC_PORT: u16 = 5900;

/// The unit that runs the browser for automation.
pub const BROWSER_UNIT: &str = "eidolon-browser.service";
/// The unit that runs the sign-in window; it pulls in the display and the
/// screen share, and stops them with it.
pub const VIEW_UNIT: &str = "eidolon-view.service";
/// The sign-in window's display.
pub const XVFB_UNIT: &str = "eidolon-xvfb.service";
/// The sign-in window's screen share.
pub const VNC_UNIT: &str = "eidolon-vnc.service";

/// Every unit, as `(file name, text with placeholders)`.
pub const UNIT_FILES: [(&str, &str); 4] = [
    (BROWSER_UNIT, include_str!("../units/eidolon-browser.service")),
    (XVFB_UNIT, include_str!("../units/eidolon-xvfb.service")),
    (VNC_UNIT, include_str!("../units/eidolon-vnc.service")),
    (VIEW_UNIT, include_str!("../units/eidolon-view.service")),
];

/// The profile's preferences for an unattended browser.
pub const USER_JS: &str = include_str!("../units/user.js");

/// The ports the units use inside the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ports {
    /// Where the browser listens for automation, on 127.0.0.1.
    pub automation: u16,
    /// Where the screen share listens, on 127.0.0.1.
    pub vnc: u16,
}

impl Default for Ports {
    fn default() -> Self {
        Self { automation: DEFAULT_AUTOMATION_PORT, vnc: DEFAULT_VNC_PORT }
    }
}

/// A unit's text with the ports filled in.
pub fn render(template: &str, ports: Ports) -> String {
    template.replace("@AUTOMATION_PORT@", &ports.automation.to_string()).replace("@VNC_PORT@", &ports.vnc.to_string())
}

/// Every unit rendered, as `(file name, text)`.
pub fn units(ports: Ports) -> Vec<(&'static str, String)> {
    UNIT_FILES.iter().map(|(name, text)| (*name, render(text, ports))).collect()
}
