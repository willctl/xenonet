// What an Authorized allowed can't be changed after decide made it, and a
// request can't be put together field by field.
use xenonet_authz::{Action, Authorized, Request};

fn widen(allowed: &mut Authorized) {
    allowed.action = Action::Admit;
}

fn rewrite(request: &mut Request) {
    request.action = Action::Admit;
}

fn main() {}
