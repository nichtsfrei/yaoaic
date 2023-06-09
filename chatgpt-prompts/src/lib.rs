//! Parses various sources of csv files following the awesome-chatgpt=prompts format:
//! ```text
//! "act","prompt"
//!
//! ```
//!
//! from various sources and combines them.

use std::fmt::Display;

use hyper::{body::Bytes, http, Body, Client, Request};
use hyper_tls::HttpsConnector;
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::AsyncReadExt};

#[derive(Debug, PartialEq, Eq)]
pub enum Source<'a> {
    Http(&'a str),
    File(&'a str),
    Raw(&'a [u8]),
}

impl<'a> Display for Source<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Http(s) => write!(f, "{s}"),
            Source::File(s) => write!(f, "{s}"),
            Source::Raw(s) => write!(f, "{}", std::str::from_utf8(s).unwrap_or_default()),
        }
    }
}

impl<'a> std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Prompt {
    pub act: String,
    pub prompt: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    FormatError(String),
    LoadError(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::FormatError(e) => write!(f, "{e}"),
            Error::LoadError(e) => write!(f, "{e}"),
        }
    }
}

impl From<http::Error> for Error {
    fn from(value: http::Error) -> Self {
        Self::LoadError(value.to_string())
    }
}

impl From<hyper::Error> for Error {
    fn from(value: hyper::Error) -> Self {
        Self::LoadError(value.to_string())
    }
}
pub struct PromptLoader {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::LoadError(format!("{value:?}"))
    }
}

impl PromptLoader {
    async fn parse_bytes(b: Vec<u8>) -> Vec<Result<Prompt>> {
        let result = tokio::task::spawn_blocking(move || {
            let mut cr = csv::Reader::from_reader(&b as &[u8]);
            cr.deserialize()
                .map(|e| e.map_err(|e| Error::FormatError(e.to_string())))
                .collect()
        })
        .await
        .unwrap_or_default();
        result
    }

    async fn load_file(p: &str) -> Result<Vec<u8>> {
        let mut file = File::open(p).await?;
        let mut contents = vec![];
        file.read_to_end(&mut contents).await?;
        Ok(contents)
    }

    async fn parse_source(source: &Source<'_>) -> Vec<Result<Prompt>> {
        match source {
            Source::Http(u) => match Self::send(u).await {
                Ok(b) => Self::parse_bytes(b.into()).await,
                Err(e) => vec![Err(e)],
            },
            Source::File(p) => match Self::load_file(p).await {
                Ok(b) => Self::parse_bytes(b).await,
                Err(e) => {
                    vec![Err(e)]
                }
            },
            Source::Raw(b) => Self::parse_bytes(b.into_iter().map(|b| *b).collect()).await,
        }
    }

    async fn send(src: &str) -> Result<Bytes> {
        let req = Request::get(src).body(Body::empty())?;
        let https = HttpsConnector::new();
        let client = Client::builder().build(https);
        let res = client.request(req).await?;
        hyper::body::to_bytes(res.into_body())
            .await
            .map_err(|e| e.into())
    }

    pub async fn load<'a>(sources: &[Source<'_>]) -> Vec<Result<Prompt>> {
        let mut result = Vec::new();
        for s in sources {
            let b = Self::parse_source(s).await;
            result.extend(b);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parse() {
        let example = r###"
"act","prompt"
"1","1"
"2","2"
"###;
        let result = PromptLoader::load(&[Source::Raw(example.as_bytes())]).await;
        let expected = vec![
            Ok(Prompt {
                act: "1".into(),
                prompt: "1".into(),
            }),
            Ok(Prompt {
                act: "2".into(),
                prompt: "2".into(),
            }),
        ];
        assert_eq!(result, expected);
    }
}
