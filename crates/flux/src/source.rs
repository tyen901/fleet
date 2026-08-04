use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result};
use flux::{
    FluxResult, ProfileFingerprint, SegmentKey, SourceLookupRequest, StoreLookupResult,
    StoreOccurrence, StoreSource, StoreSourceRef,
};
use futures_util::{future::BoxFuture, FutureExt};
use object_store::http::HttpBuilder;
use object_store::{ClientOptions, ObjectStore};
use percent_encoding::percent_decode_str;
use url::Url;

struct SwiftyStoreSource {
    source_id: String,
    store: Arc<dyn ObjectStore>,
    hits_by_key: HashMap<SegmentKey, Vec<StoreOccurrence>>,
}

pub(crate) fn build_store_sources(index: crate::SwiftyStoreIndex) -> Result<Vec<StoreSourceRef>> {
    let mut by_base_url = BTreeMap::<String, Vec<crate::SwiftyStoreObject>>::new();
    for object in index.objects {
        by_base_url
            .entry(store_base_url(&object)?)
            .or_default()
            .push(object);
    }

    let mut sources = Vec::with_capacity(by_base_url.len());
    for (base_url, objects) in by_base_url {
        let source_id = format!("swifty:{base_url}");
        let store = HttpBuilder::new()
            .with_url(base_url.clone())
            .with_client_options(ClientOptions::new().with_allow_http(true))
            .build()
            .context("build Swifty HTTP object store")?;

        let mut hits_by_key = HashMap::new();
        for object in objects {
            for part in object.parts {
                let occurrence = StoreOccurrence {
                    source_id: source_id.clone(),
                    object: object.object_path.clone(),
                    object_range: part.object_range.clone(),
                    key: part.key.clone(),
                    validation: part.validation.clone(),
                };
                hits_by_key
                    .entry(part.key)
                    .or_insert_with(Vec::new)
                    .push(occurrence);
            }
        }

        for hits in hits_by_key.values_mut() {
            hits.sort_by(|a, b| {
                a.object
                    .as_ref()
                    .cmp(b.object.as_ref())
                    .then_with(|| a.object_range.start.cmp(&b.object_range.start))
                    .then_with(|| a.object_range.end.cmp(&b.object_range.end))
            });
        }

        sources.push(Arc::new(SwiftyStoreSource {
            source_id,
            store: Arc::new(store),
            hits_by_key,
        }) as StoreSourceRef);
    }

    Ok(sources)
}

impl StoreSource for SwiftyStoreSource {
    fn id(&self) -> &str {
        &self.source_id
    }

    fn object_store(&self) -> Arc<dyn ObjectStore> {
        self.store.clone()
    }

    fn lookup_many<'a>(
        &'a self,
        profile: ProfileFingerprint,
        requests: &'a [SourceLookupRequest],
        limit_per_key: usize,
    ) -> BoxFuture<'a, FluxResult<Vec<StoreLookupResult>>> {
        async move {
            if profile != crate::swifty_profile_fingerprint() {
                return Ok(requests
                    .iter()
                    .map(|req| StoreLookupResult {
                        key: req.key.clone(),
                        hits: Vec::new(),
                    })
                    .collect());
            }

            let mut results = Vec::with_capacity(requests.len());
            for request in requests {
                let mut hits = Vec::new();
                if let Some(candidates) = self.hits_by_key.get(&request.key) {
                    for hit in candidates {
                        if hit.validation == request.validation
                            && (hit.object_range.end - hit.object_range.start) == request.len
                        {
                            hits.push(hit.clone());
                            if hits.len() >= limit_per_key {
                                break;
                            }
                        }
                    }
                }
                results.push(StoreLookupResult {
                    key: request.key.clone(),
                    hits,
                });
            }
            Ok(results)
        }
        .boxed()
    }
}

fn store_base_url(object: &crate::SwiftyStoreObject) -> Result<String> {
    let url = Url::parse(&object.source_url)?;
    let suffix = object.object_path.as_ref();
    let path = percent_decode_str(url.path())
        .decode_utf8()
        .context("decode source_url path")?;

    if !path.ends_with(suffix) {
        return Err(anyhow::anyhow!(
            "source_url path {} does not end with object_path {}",
            path,
            suffix
        ));
    }

    let base_path = path[..path.len() - suffix.len()].to_string();

    let mut base_url = url;
    base_url.set_path(&base_path);
    base_url.set_query(None);
    base_url.set_fragment(None);
    Ok(base_url.to_string())
}

#[cfg(test)]
mod tests {
    use flux::{
        OpaqueSegmentIdentity, ProfileFingerprint, SegmentKey, SourceLookupRequest, TargetPath,
        ValidationSpec,
    };
    use object_store::path::Path as ObjectPath;

    use super::*;

    #[test]
    fn empty_store_index_creates_no_sources() {
        let sources = build_store_sources(crate::SwiftyStoreIndex {
            objects: Vec::new(),
        })
        .expect("build sources");

        assert!(sources.is_empty());
    }

    #[test]
    fn store_sources_are_grouped_by_base_url() {
        let index = crate::SwiftyStoreIndex {
            objects: vec![
                object("one", "https://a.example/mods/one.pbo", "mods/one.pbo"),
                object("two", "https://b.example/mods/two.pbo", "mods/two.pbo"),
                object(
                    "three",
                    "https://a.example/mods/three.pbo",
                    "mods/three.pbo",
                ),
            ],
        };

        let sources = build_store_sources(index).expect("build sources");
        let ids = sources
            .iter()
            .map(|source| source.id().to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                "swifty:https://a.example/".to_string(),
                "swifty:https://b.example/".to_string()
            ]
        );
    }

    #[test]
    fn source_url_object_path_mismatch_is_rejected() {
        let index = crate::SwiftyStoreIndex {
            objects: vec![object(
                "one",
                "https://a.example/mods/one.pbo",
                "other/one.pbo",
            )],
        };

        assert!(build_store_sources(index).is_err());
    }

    #[test]
    fn percent_encoded_source_url_path_matches_decoded_object_path() {
        let index = crate::SwiftyStoreIndex {
            objects: vec![object(
                "one",
                "https://a.example/@rksl/docs/home%20-%20rksl%20studios%20community.url",
                "@rksl/docs/home - rksl studios community.url",
            )],
        };

        let sources = build_store_sources(index).expect("build sources");

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id(), "swifty:https://a.example/");
    }

    #[tokio::test]
    async fn lookup_many_returns_request_order_buckets() {
        let first = key(1, 1);
        let second = key(2, 1);
        let source = only_source(vec![
            object_with_key(
                "one",
                "https://a.example/mods/one.pbo",
                "mods/one.pbo",
                first.clone(),
            ),
            object_with_key(
                "two",
                "https://a.example/mods/two.pbo",
                "mods/two.pbo",
                second.clone(),
            ),
        ]);
        let requests = vec![request(second.clone()), request(first.clone())];

        let results = source
            .lookup_many(crate::swifty_profile_fingerprint(), &requests, 10)
            .await
            .expect("lookup");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].key, second);
        assert_eq!(results[0].hits.len(), 1);
        assert_eq!(results[1].key, first);
        assert_eq!(results[1].hits.len(), 1);
    }

    #[tokio::test]
    async fn lookup_many_respects_limit_per_key() {
        let key = key(1, 1);
        let source = only_source(vec![
            object_with_key(
                "two",
                "https://a.example/mods/two.pbo",
                "mods/two.pbo",
                key.clone(),
            ),
            object_with_key(
                "one",
                "https://a.example/mods/one.pbo",
                "mods/one.pbo",
                key.clone(),
            ),
        ]);
        let requests = vec![request(key)];

        let results = source
            .lookup_many(crate::swifty_profile_fingerprint(), &requests, 1)
            .await
            .expect("lookup");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].hits.len(), 1);
        assert_eq!(results[0].hits[0].object.as_ref(), "mods/one.pbo");
    }

    fn only_source(objects: Vec<crate::SwiftyStoreObject>) -> StoreSourceRef {
        let sources =
            build_store_sources(crate::SwiftyStoreIndex { objects }).expect("build sources");
        assert_eq!(sources.len(), 1);
        sources.into_iter().next().expect("source")
    }

    fn request(key: SegmentKey) -> SourceLookupRequest {
        SourceLookupRequest {
            validation: ValidationSpec {
                profile: key.profile,
                key: key.clone(),
                len: key.len,
            },
            len: key.len,
            key,
        }
    }

    fn object(target: &str, source_url: &str, object_path: &str) -> crate::SwiftyStoreObject {
        object_with_key(target, source_url, object_path, key(1, 1))
    }

    fn object_with_key(
        target: &str,
        source_url: &str,
        object_path: &str,
        key: SegmentKey,
    ) -> crate::SwiftyStoreObject {
        let validation = ValidationSpec {
            profile: key.profile,
            key: key.clone(),
            len: 1,
        };
        crate::SwiftyStoreObject {
            target_path: TargetPath::new(format!("{target}.pbo")).expect("target path"),
            source_url: source_url.to_string(),
            object_path: ObjectPath::from(object_path),
            parts: vec![crate::SwiftyStorePart {
                key,
                validation,
                object_range: 0..1,
                target_range: 0..1,
            }],
        }
    }

    fn key(id: u8, len: u64) -> SegmentKey {
        flux::SegmentKey::new(
            ProfileFingerprint::new([id; 32]),
            OpaqueSegmentIdentity::new(vec![id]).expect("identity"),
            len,
        )
        .expect("segment key")
    }
}
