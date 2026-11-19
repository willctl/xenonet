//! The arguments of each tool, which are also the tools' JSON Schemas.
//! The names follow the Firefox MCP server that agents already use, so
//! skills written for it carry over.

use schemars::JsonSchema;
use serde::Deserialize;

/// No arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewPage {
    /// The address to open. Leave out for a blank page.
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageId {
    /// The page's id, from list_pages.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavigatePage {
    /// The address to open: http, https or about:blank.
    pub url: String,
}

/// Which way to go in the page's history.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Back,
    Forward,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavigateHistory {
    /// back or forward.
    pub direction: Direction,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Uid {
    /// An element's uid from the last take_snapshot.
    pub uid: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FillByUid {
    /// An element's uid from the last take_snapshot.
    pub uid: String,
    /// The text to put in the field, or the option to choose in a select.
    pub value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypeText {
    /// The text to type into the element that has focus.
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PressKey {
    /// A key name such as Enter, Tab, Escape or ArrowDown, a character, or a combination such as Control+a.
    pub key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadFileByUid {
    /// The file input's uid from the last take_snapshot.
    pub uid: String,
    /// The file to upload: a path inside the shared files folder, absolute or relative to it.
    #[serde(rename = "filePath", alias = "host_path")]
    pub file_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FetchDownload {
    /// The file's name from list_downloads.
    pub name: String,
}
