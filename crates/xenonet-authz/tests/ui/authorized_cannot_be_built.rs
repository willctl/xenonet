// Only decide makes an Authorized, a request's checked person or a
// checked presence.
use xenonet_authz::{Action, Authorized, PersonSigned, VerifiedPresence};

fn main() {
    let _allowed = Authorized { action: Action::Admit };
    let _person = PersonSigned { act: [0; 32] };
    let _present = VerifiedPresence { act: [0; 32] };
}
