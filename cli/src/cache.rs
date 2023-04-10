//! This crate provides a caching mechanism for storing and retrieving values from disk.
//!
//! It allows users to store and retrieve values in a cache directory.
//! The cache directory can be specified during initialization.
//! The cache can be enabled or disabled, and the maximum age of the cache can be set.
//! The crate provides functions to load cached values, store values in the cache, and retrieve values from the cache.
use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::toml_file as toml;
use anyhow::{bail, Context, Result};

/// Initializes the cache.
///
/// Before returning the cache the given dir is verified and if it does not exist than it will be
/// created.
pub fn init<P>(dir: P, max_cache_age: Duration) -> Result<Cache<P>>
where
    P: AsRef<Path>,
{
    Cache::init(dir, max_cache_age)
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
/// Represents a cached value.
///
/// It contains two fields: `created` and `value`. The `created` field is of type `Duration` and represents the time when the value was created. The `value` field is of type `T` and represents the cached value.
pub struct Value<T>
where
    T: serde::Serialize,
{
    /// When this value is created.
    ///
    /// Is used to identify the age of the value, when duration is higher than the `max_age_cache`
    /// of the cache the value will not be used.
    created: Duration,
    /// The value to cache.
    ///
    /// T must be Serialize so that serde can use it.
    value: T,
}

impl<T> From<T> for Value<T>
where
    T: serde::Serialize,
{
    fn from(value: T) -> Self {
        let start = SystemTime::now();
        let created = start
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");
        Self { created, value }
    }
}

/// Handles the cache dir.
pub struct Cache<P> {
    /// The directory to work in.
    dir: P,
    max_cache_age: Duration,
}

fn check_or_create_dir<P>(dir: P) -> Result<()>
where
    P: AsRef<Path>,
{
    if let Ok(exist) = fs::metadata(dir.as_ref()) {
        if !exist.is_dir() {
            bail!(
                "{} exists but it is not a dir.",
                &dir.as_ref().to_str().unwrap_or_default()
            );
        }
        Ok(())
    } else {
        fs::create_dir(dir.as_ref()).with_context(|| {
            format!(
                "unable to create dir {}",
                &dir.as_ref().to_str().unwrap_or_default()
            )
        })
    }
}

impl<P> Cache<P>
where
    P: AsRef<Path>,
{
    /// Verifies if given dir exists or creates it before returning a Cache
    fn init(dir: P, max_cache_age: Duration) -> Result<Self> {
        check_or_create_dir(&dir)?;
        Ok(Self { dir, max_cache_age })
    }

    /// Loads a cached value from the cache directory.
    ///
    /// The `file_name` parameter specifies the name of the file to load from the cache directory.
    /// The function returns `Ok(Some(T))` if the file exists in the cache directory and its age is less than the maximum cache age. Otherwise, it returns `Ok(None)`
    pub async fn load_cached<T>(&self, file_name: &str) -> Result<Option<T>>
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        let mut path = PathBuf::from(self.dir.as_ref());
        path.push(file_name);
        let cached: Value<T> = toml::load(path).await?;
        let created = cached.created;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
        if now - created < self.max_cache_age {
            Ok(Some(cached.value))
        } else {
            Ok(None)
        }
    }

    /// Stores a value in the cache directory.
    ///
    /// The `file_name` parameter specifies the name of the file to store in the cache directory.
    /// The `to_cache` parameter specifies the value to store in the cache directory.
    pub async fn store_cache<T>(&self, file_name: &str, to_cache: T) -> Result<()>
    where
        T: serde::ser::Serialize,
    {
        let mut path = PathBuf::from(self.dir.as_ref());
        path.push(file_name);
        toml::replace(path, to_cache).await
    }

    /// Retrieves a value from the cache directory, or loads it if it does not exist.
    ///
    /// The `file_name` parameter specifies the name of the file to retrieve from the cache directory.
    /// The `input` parameter specifies the input to the loader function.
    /// The `loader` parameter is a closure that takes an input and returns a future that resolves to a result of type `T`.
    ///
    /// The function first attempts to load the value from the cache directory using the `load_cached` function.
    /// If the value exists in the cache directory and its age is less than the maximum cache age, the function returns the cached value.
    /// Otherwise, the function calls the `loader` closure with the `input` parameter to load the value.
    /// The function then stores the loaded value in the cache directory using the `store_cache` function and returns the loaded value.
    pub async fn with_cached<F, T, I>(
        &self,
        file_name: &str,
        input: I,
        mut loader: impl FnMut(I) -> F,
    ) -> Result<T>
    where
        T: Serialize + serde::de::DeserializeOwned + Sized,
        F: Future<Output = Result<T>>,
    {
        match self.load_cached::<T>("prompts.toml").await {
            Ok(Some(x)) => Ok(x),
            Ok(None) | Err(_) => {
                let r = loader(input).await?;
                let cached: Value<T> = r.into();
                self.store_cache(file_name, &cached).await?;
                Ok(cached.value)
            }
        }
    }
}
