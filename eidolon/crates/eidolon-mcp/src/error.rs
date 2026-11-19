//! The tools' errors. Each message is a plain sentence that says what to do.

use std::fmt;

/// Why a tool did not do what was asked.
#[derive(Debug)]
pub enum Error {
    /// The browser's protocol failed or the browser said no.
    Bidi(eidolon_bidi::Error),
    /// Reaching the browser in its cell failed.
    Browser(String),
    /// The cell's driver failed.
    Driver(String),
    /// A rule refused the request. The text says which and what to do.
    Refused(String),
    /// A uid that the last snapshot of the page did not hand out.
    UnknownUid(String),
    /// The element is gone: the page changed after the snapshot.
    StaleElement,
    /// There is no page open.
    NoPage,
    /// The selected page was closed or is gone.
    PageGone,
    /// There is no page with this id.
    NoSuchPage(String),
    /// The tool's arguments do not fit its schema.
    BadArguments(String),
    /// There is no finished download of this name.
    NoSuchDownload(String),
    /// A file on the host could not be used.
    Io(String),
    /// The page's snapshot came back in a shape we do not know.
    BadSnapshot(String),
    /// The MCP server stopped with an error.
    Serve(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bidi(e) => write!(f, "{e}"),
            Self::Browser(e) | Self::Driver(e) | Self::Refused(e) | Self::BadArguments(e) | Self::Io(e) => write!(f, "{e}"),
            Self::UnknownUid(uid) => write!(f, "There is no element {uid} in the last snapshot of this page. Call take_snapshot and use a uid from it."),
            Self::StaleElement => write!(f, "The element is no longer on the page. Call take_snapshot again and use a new uid."),
            Self::NoPage => write!(f, "There is no page open. Call new_page to open one."),
            Self::PageGone => write!(f, "The selected page is gone. Call list_pages, then select_page."),
            Self::NoSuchPage(id) => write!(f, "There is no page {id}. Call list_pages to see the open pages."),
            Self::NoSuchDownload(name) => write!(f, "There is no finished download named {name}. Call list_downloads to see what is there."),
            Self::BadSnapshot(e) => write!(f, "The page's snapshot has an unexpected shape: {e}"),
            Self::Serve(e) => write!(f, "The MCP server stopped: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<eidolon_bidi::Error> for Error {
    fn from(e: eidolon_bidi::Error) -> Self {
        Self::Bidi(e)
    }
}

impl From<adytum_driver::DriverError> for Error {
    fn from(e: adytum_driver::DriverError) -> Self {
        let (_, what, fix) = e.parts();
        Self::Driver(format!("{what} {fix}"))
    }
}

/// This crate's result.
pub type Result<T> = std::result::Result<T, Error>;
