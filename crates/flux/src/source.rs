use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use flux::{
    FluxResult, ProfileFingerprint, SegmentKey, SourceLookupRequest, StoreLookupResult,
    StoreOccurrence, StoreSource, StoreSourceRef,
};
use futures_util::{future::BoxFuture, FutureExt};
use object_store::http::HttpBuilder;
use object_store::{ClientOptions, ObjectStore};

use crate::input::{swifty_profile_fingerprint, SwiftyStoreIndex};

struct SwiftyStoreSource {
    source_id: String,
    store: Arc<dyn ObjectStore>,
    hits_by_key: HashMap<SegmentKey, Vec<StoreOccurrence>>,
}

pub(crate) fn build_store_sources(index: SwiftyStoreIndex) -> Result<Vec<StoreSourceRef>> {
    if index.objects.is_empty() {
        return Ok(Vec::new());
    }

    let source_id = format!("swifty:{}", index.base_url);
    let store_url = index.base_url.trim_end_matches('/');
    let store = HttpBuilder::new()
        .with_url(store_url)
        .with_client_options(ClientOptions::new().with_allow_http(true))
        .build()
        .context("build Swifty HTTP object store")?;
    let mut hits_by_key = HashMap::new();
    for object in index.objects {
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

    Ok(vec![Arc::new(SwiftyStoreSource {
        source_id,
        store: Arc::new(store),
        hits_by_key,
    }) as StoreSourceRef])
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
            if profile != swifty_profile_fingerprint() {
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
                if limit_per_key > 0 {
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::http::Uri;
    use axum::routing::get;
    use axum::{body::Body, response::Response, Router};
    use flux::{
        OpaqueSegmentIdentity, ProfileFingerprint, SegmentKey, SourceLookupRequest, ValidationSpec,
    };
    use object_store::path::Path as ObjectPath;
    use object_store::ObjectStoreExt;

    use super::*;
    use crate::input::{
        swifty_profile_fingerprint, SwiftyStoreIndex, SwiftyStoreObject, SwiftyStorePart,
    };

    #[test]
    fn empty_store_index_creates_no_sources() {
        let sources = build_store_sources(SwiftyStoreIndex {
            base_url: "https://a.example/base/".to_string(),
            objects: Vec::new(),
        })
        .expect("build sources");

        assert!(sources.is_empty());
    }

    #[tokio::test]
    async fn store_source_escapes_special_object_names_under_encoded_base_path() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/{*path}", get(record_request))
            .with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind server");
        let address = listener.local_addr().expect("server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve requests");
        });
        let object_path = "@mod/addons/a % #.pbo";
        let sources = build_store_sources(SwiftyStoreIndex {
            base_url: format!("http://{address}/base%20path/"),
            objects: vec![object(object_path)],
        })
        .expect("build source");

        sources[0]
            .object_store()
            .get(&ObjectPath::parse(object_path).expect("object path"))
            .await
            .expect("get object");

        assert_eq!(
            requests.lock().expect("requests lock").as_slice(),
            ["/base%20path/@mod/addons/a%20%25%20%23.pbo"]
        );
        server.abort();
    }

    #[tokio::test]
    async fn lookup_many_returns_request_order_buckets() {
        let first = key(1, 1);
        let second = key(2, 1);
        let source = only_source(vec![
            object_with_key("mods/one.pbo", first.clone()),
            object_with_key("mods/two.pbo", second.clone()),
        ]);
        let requests = vec![request(second.clone()), request(first.clone())];

        let results = source
            .lookup_many(swifty_profile_fingerprint(), &requests, 10)
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
            object_with_key("mods/two.pbo", key.clone()),
            object_with_key("mods/one.pbo", key.clone()),
        ]);
        let requests = vec![request(key)];

        let results = source
            .lookup_many(swifty_profile_fingerprint(), &requests, 1)
            .await
            .expect("lookup");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].hits.len(), 1);
        assert_eq!(results[0].hits[0].object.as_ref(), "mods/one.pbo");
    }

    #[tokio::test]
    async fn lookup_many_returns_no_hits_when_limit_is_zero() {
        let key = key(1, 1);
        let source = only_source(vec![object_with_key("mods/one.pbo", key.clone())]);
        let results = source
            .lookup_many(swifty_profile_fingerprint(), &[request(key)], 0)
            .await
            .expect("lookup");

        assert!(results[0].hits.is_empty());
    }

    fn only_source(objects: Vec<SwiftyStoreObject>) -> StoreSourceRef {
        let sources = build_store_sources(SwiftyStoreIndex {
            base_url: "https://a.example/".to_string(),
            objects,
        })
        .expect("build sources");
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

    fn object(object_path: &str) -> SwiftyStoreObject {
        object_with_key(object_path, key(1, 1))
    }

    fn object_with_key(object_path: &str, key: SegmentKey) -> SwiftyStoreObject {
        let validation = ValidationSpec {
            profile: key.profile,
            key: key.clone(),
            len: 1,
        };
        SwiftyStoreObject {
            object_path: ObjectPath::parse(object_path).expect("object path"),
            parts: vec![SwiftyStorePart {
                key,
                validation,
                object_range: 0..1,
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

    async fn record_request(State(requests): State<Arc<Mutex<Vec<String>>>>, uri: Uri) -> Response {
        requests
            .lock()
            .expect("requests lock")
            .push(uri.path().to_string());
        Response::new(Body::from("object"))
    }
}
