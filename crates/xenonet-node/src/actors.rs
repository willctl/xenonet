//! xenonet-actors for the twin of core/actors/tokens.js.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use xenonet_actors::tokens;

#[napi(object)]
pub struct NewToken {
    pub token: String,
    pub hash: String,
}

#[napi(js_name = "tokenPatterns")]
pub fn token_patterns() -> Vec<&'static str> {
    vec![tokens::TOKEN_PREFIX, tokens::TOKEN_PATTERN, tokens::GATEWAY_PREFIX, tokens::GATEWAY_PATTERN, tokens::TOKEN_GREP]
}

#[napi(js_name = "isToken")]
pub fn is_token(s: String) -> bool {
    tokens::is_token(&s)
}

#[napi(js_name = "isGatewayToken")]
pub fn is_gateway_token(s: String) -> bool {
    tokens::is_gateway_token(&s)
}

#[napi(js_name = "hashToken")]
pub fn hash_token(token: String) -> String {
    tokens::hash_token(&token)
}

fn made(r: std::result::Result<(String, String), getrandom_error::Error>) -> Result<NewToken> {
    r.map(|(token, hash)| NewToken { token, hash }).map_err(|e| Error::new(Status::GenericFailure, format!("no random bytes: {e}")))
}

mod getrandom_error {
    pub type Error = getrandom::Error;
}

#[napi(js_name = "newToken")]
pub fn new_token() -> Result<NewToken> {
    made(tokens::new_token())
}

#[napi(js_name = "newGatewayToken")]
pub fn new_gateway_token() -> Result<NewToken> {
    made(tokens::new_gateway_token())
}

#[napi(js_name = "bearerOf")]
pub fn bearer_of(raw: String) -> Option<String> {
    tokens::bearer_of(&raw).map(str::to_string)
}

/// Whether a URL carries a token: its search params as [key, value], and
/// its path.
#[napi(js_name = "tokenInUrl")]
pub fn token_in_url(params: Vec<Vec<String>>, path: String) -> bool {
    tokens::token_in_url(params.iter().map(|p| (p.first().map_or("", String::as_str), p.get(1).map_or("", String::as_str))), &path)
}
