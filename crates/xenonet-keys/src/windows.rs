//! Windows Hello as the person's presence check (docs/design/direction.md,
//! "Open questions for Will", question 1). Windows Hello always lets a PIN
//! stand in for a face or a finger, and an agent that can type could enter
//! a PIN, so a device that checks this way is weaker, like Linux's polkit
//! or passphrase, and its entry in the person's chain says so. Will
//! accepted that on 2026-11-10.
//!
//! The prompt is UserConsentVerifier's. A desktop program shows it over a
//! window through the interop interface, here the window in front, which
//! is XenoNet's when the person has just acted there; without one it falls
//! back to the plain call, which Windows shows on its own.

use windows::core::{factory, HSTRING};
use windows::Security::Credentials::UI::{UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability};
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows_future::IAsyncOperation;
use xenonet_id::presence::UserCheck;
use xenonet_id::{IdError, R};

/// Asks Windows Hello: a face, a finger or the PIN.
pub struct HelloCheck;

impl HelloCheck {
    /// The check, when Windows Hello is set up for this user. Asking
    /// whether it is shows nothing.
    pub fn available() -> Option<HelloCheck> {
        let availability = UserConsentVerifier::CheckAvailabilityAsync().and_then(|op| op.join()).ok()?;
        (availability == UserConsentVerifierAvailability::Available).then_some(HelloCheck)
    }
}

impl UserCheck for HelloCheck {
    #[expect(unsafe_code, reason = "GetForegroundWindow and the interop call, which the windows crate marks unsafe")]
    fn confirm(&self, reason: &str) -> R<()> {
        let message = HSTRING::from(format!("XenoNet: {reason}"));
        // SAFETY: GetForegroundWindow takes nothing and returns a handle
        // or null; the handle is only passed on to Windows.
        let window = unsafe { GetForegroundWindow() };
        let op = if window.is_invalid() {
            UserConsentVerifier::RequestVerificationAsync(&message)
        } else {
            factory::<UserConsentVerifier, IUserConsentVerifierInterop>().and_then(|interop| {
                // SAFETY: the window handle came from Windows just now, and
                // the type asked for is the operation this call returns.
                unsafe { interop.RequestVerificationForWindowAsync::<IAsyncOperation<UserConsentVerificationResult>>(window, &message) }
            })
        };
        let result = op.and_then(|op| op.join()).map_err(|e| IdError::internal(format!("Windows Hello didn't ask: {e}")))?;
        if result == UserConsentVerificationResult::Verified {
            Ok(())
        } else {
            Err(IdError::refused("Windows Hello didn't confirm it was you"))
        }
    }
}
