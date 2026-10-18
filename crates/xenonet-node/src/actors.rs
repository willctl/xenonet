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

// ----- token files -----

use crate::base::{outcome, runner, text, RunAnswer};
use crate::js::Jv;
use serde_json::{json, Value};
use xenonet_actors::token_file;

#[napi(js_name = "heavyFolders")]
pub fn heavy_folders() -> Vec<&'static str> {
    token_file::HEAVY_FOLDERS.to_vec()
}

#[napi(js_name = "readTokenFile")]
pub fn read_token_file(file: String, windows: bool, run: Function<'_, Vec<String>, RunAnswer>) -> Jv {
    outcome(token_file::read_token_file(&file, windows, &mut runner(&run)).map(text))
}

/// The twin hands over the two variables, each when it's set and not empty.
#[napi(js_name = "tokenFromEnv")]
pub fn token_from_env(token_file_var: Option<String>, token_var: Option<String>, windows: bool, run: Function<'_, Vec<String>, RunAnswer>) -> Jv {
    let env = |k: &str| match k {
        "XENONET_ACTOR_TOKEN_FILE" => token_file_var.clone(),
        "XENONET_ACTOR_TOKEN" => token_var.clone(),
        _ => None,
    };
    let found = token_file::token_from_env(&env, windows, &mut runner(&run));
    outcome(found.map(|t| t.map_or(Jv::Json(Value::Null), |(token, file)| Jv::Obj(vec![("token", text(token)), ("file", Jv::Json(json!(file)))]))))
}

#[napi(js_name = "gitTreeOf")]
pub fn git_tree_of(dir: String) -> Option<String> {
    token_file::git_tree_of(&dir)
}

#[napi(js_name = "tokenFileRefusal")]
pub fn token_file_refusal(file: String, force: bool, projects_dir: Option<String>) -> Option<String> {
    token_file::token_file_refusal(&file, force, projects_dir.as_deref())
}

#[napi(js_name = "writeTokenFile")]
pub fn write_token_file(file: String, token: String, windows: bool, run: Function<'_, Vec<String>, RunAnswer>, force: bool, projects_dir: Option<String>) -> Jv {
    outcome(token_file::write_token_file(&file, &token, windows, &mut runner(&run), force, projects_dir.as_deref()).map(text))
}
