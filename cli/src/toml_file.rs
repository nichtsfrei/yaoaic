use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use tokio::io::AsyncReadExt;
use tokio::{fs::File, io::AsyncWriteExt};

// TODO use tokio fs
pub async fn replace<P, T>(path: P, to_cache: T) -> Result<()>
where
    P: AsRef<Path>,
    T: serde::ser::Serialize,
{
    let cached_toml =
        toml::to_string_pretty(&to_cache).context("unable to wrote cached prompts toml")?;
    let mut file = File::create(path).await?;
    file.write_all(cached_toml.as_bytes()).await?;
    Ok(())
}

pub async fn load<P, T>(path: P) -> Result<T>
where
    T: Serialize + serde::de::DeserializeOwned,
    P: AsRef<Path>,
{
    let mut f = File::open(path.as_ref()).await.with_context(|| {
        format!(
            "{} unable to open.",
            path.as_ref().to_str().unwrap_or_default()
        )
    })?;
    let mut cached = String::new();
    f.read_to_string(&mut cached)
        .await
        .context("unable to load into string")?;
    let cached: T = toml::from_str(&cached).with_context(|| {
        format!(
            "{} has unknown format.",
            path.as_ref().to_str().unwrap_or_default()
        )
    })?;
    Ok(cached)
}
