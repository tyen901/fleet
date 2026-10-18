use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use flux::{ContentKey, ContentSource, SourceRange};
use futures_util::{future::BoxFuture, FutureExt};
use object_store::http::HttpBuilder;
use object_store::{ClientOptions, ObjectStore};

use crate::input::SwiftyStoreIndex;

struct SwiftyStoreSource {
    store: Arc<dyn ObjectStore>,
    hits_by_key: HashMap<ContentKey, SourceRange>,
}

pub(crate) fn build_store_sources(index: SwiftyStoreIndex) -> Result<Vec<Arc<dyn ContentSource>>> {
    if index.objects.is_empty() {
        return Ok(Vec::new());
    }
    let store_url = index.base_url.strip_suffix('/').unwrap_or(&index.base_url);
    let store = HttpBuilder::new()
        .with_url(store_url)
        .with_client_options(ClientOptions::new().with_allow_http(true))
        .build()
        .context("build Swifty HTTP object store")?;
    let mut hits_by_key: HashMap<ContentKey, SourceRange> = HashMap::new();
    for object in index.objects {
        for part in object.parts {
            let candidate = SourceRange {
                path: object.object_path.clone(),
                offset: part.offset,
            };
            hits_by_key
                .entry(part.key)
                .and_modify(|hit| {
                    if (candidate.path.as_ref(), candidate.offset) < (hit.path.as_ref(), hit.offset)
                    {
                        *hit = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
    }
    Ok(vec![Arc::new(SwiftyStoreSource {
        store: Arc::new(store),
        hits_by_key,
    }) as Arc<dyn ContentSource>])
}

impl ContentSource for SwiftyStoreSource {
    fn object_store(&self) -> Arc<dyn ObjectStore> {
        self.store.clone()
    }

    fn lookup<'a>(
        &'a self,
        keys: &'a [ContentKey],
        _limit_per_key: usize,
    ) -> BoxFuture<'a, flux::Result<Vec<Vec<SourceRange>>>> {
        async move {
            Ok(keys
                .iter()
                .map(|key| self.hits_by_key.get(key).cloned().into_iter().collect())
                .collect())
        }
        .boxed()
    }
}
