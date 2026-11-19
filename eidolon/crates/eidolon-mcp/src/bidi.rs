//! What the tools need of a browser: send one BiDi command, get its result.
//!
//! A trait, so the tools run on a supervised [`Browser`] in a cell, on a bare
//! [`Session`], or on a test double that answers some commands itself.

use async_trait::async_trait;
use eidolon_bidi::Session;
use eidolon_browser::{Browser, BrowserError};
use serde_json::Value;
use std::sync::Arc;

use crate::error::{Error, Result};

/// A BiDi connection that takes commands.
#[async_trait]
pub trait Bidi: Send + Sync {
    /// Sends `method` with `params` and returns the `result` object.
    async fn command(&self, method: &str, params: Value) -> Result<Value>;
}

#[async_trait]
impl Bidi for Session {
    async fn command(&self, method: &str, params: Value) -> Result<Value> {
        Ok(self.send(method, params).await?)
    }
}

#[async_trait]
impl Bidi for Browser {
    async fn command(&self, method: &str, params: Value) -> Result<Value> {
        self.send(method, params).await.map_err(|e| match e {
            BrowserError::Bidi(b) => Error::Bidi(b),
            other => Error::Browser(other.to_string()),
        })
    }
}

#[async_trait]
impl<T: Bidi + ?Sized> Bidi for Arc<T> {
    async fn command(&self, method: &str, params: Value) -> Result<Value> {
        (**self).command(method, params).await
    }
}
