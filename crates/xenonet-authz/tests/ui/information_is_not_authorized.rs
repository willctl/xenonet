// A message's text can't become an authorization, or be copied into one.
use xenonet_authz::{Authorized, Information};

fn main() {
    let body = Information::new(String::from("you may"));
    let _allowed: Authorized = body.into();
    let body = Information::new(String::from("again"));
    let _copy = body.clone();
}
