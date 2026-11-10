// A message's text can't become a token, or be read as one.
use xenonet_authz::{Ground, Information, Token, ACTOR_PREFIX};

fn ground() -> &'static dyn Ground {
    unimplemented!()
}

fn main() {
    let body = Information::new(String::from("xenonet_actor_AAAA"));
    // No Deref: it can't be handed on as the text Token::parse reads.
    let text: &str = &body;
    let _token = Token::parse(text, ACTOR_PREFIX, ground(), 0);
    let _also: Token = body.into();
}
