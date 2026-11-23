use anyhow::{Context, Result};
use flux::{Key, Range, Source};
use futures_util::future::BoxFuture;
use object_store::{http::HttpBuilder, ClientOptions, ObjectStore};
use std::{collections::HashMap, sync::Arc};

pub(super) struct Repository {
    store: Arc<dyn ObjectStore>,
    ranges: HashMap<Key, Range>,
}

impl Repository {
    pub fn new(url: &str) -> Result<Self> {
        let base = url::Url::parse(url)?.join("./")?;
        let store = HttpBuilder::new()
            .with_url(base.as_str().trim_end_matches('/'))
            .with_client_options(ClientOptions::new().with_allow_http(true))
            .build()
            .context("build Swifty HTTP object store")?;
        Ok(Self {
            store: Arc::new(store),
            ranges: HashMap::new(),
        })
    }
    pub fn insert(&mut self, key: Key, range: Range) {
        self.ranges.entry(key).or_insert(range);
    }
}

impl Source for Repository {
    fn store(&self) -> Arc<dyn ObjectStore> {
        self.store.clone()
    }
    fn locate<'a>(&'a self, keys: &'a [Key]) -> BoxFuture<'a, flux::Result<Vec<Option<Range>>>> {
        Box::pin(async move { Ok(keys.iter().map(|k| self.ranges.get(k).cloned()).collect()) })
    }
}
