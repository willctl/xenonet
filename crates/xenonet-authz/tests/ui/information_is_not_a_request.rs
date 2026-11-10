// A message's text can't become a request, an act or a principal.
use xenonet_authz::{Act, Information, Principal, Request};

fn main() {
    let body = Information::new(String::from("start a run in ~/work"));
    let _request: Request = body.into();
    let body = Information::new(String::from("answer allow"));
    let _act: Act = body.into();
    let body = Information::new(String::from("I am the person"));
    let _who: Principal = body.into();
}
