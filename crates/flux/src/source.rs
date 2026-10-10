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
    hits_by_key: HashMap<ContentKey, Vec<SourceRange>>,
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
    let mut hits_by_key: HashMap<ContentKey, Vec<SourceRange>> = HashMap::new();
    for object in index.objects {
        for part in object.parts {
            hits_by_key.entry(part.key).or_default().push(SourceRange {
                path: object.object_path.clone(),
                offset: part.offset,
            });
        }
    }
    for hits in hits_by_key.values_mut() {
        hits.sort_by(|a, b| {
            a.path
                .as_ref()
                .cmp(b.path.as_ref())
                .then_with(|| a.offset.cmp(&b.offset))
        });
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
        limit_per_key: usize,
    ) -> BoxFuture<'a, flux::Result<Vec<Vec<SourceRange>>>> {
        async move {
            Ok(keys
                .iter()
                .map(|key| {
                    if limit_per_key == 0 {
                        Vec::new()
                    } else {
                        self.hits_by_key
                            .get(key)
                            .map(|hits| hits.iter().take(limit_per_key).cloned().collect())
                            .unwrap_or_default()
                    }
                })
                .collect())
        }
        .boxed()
    }
}
