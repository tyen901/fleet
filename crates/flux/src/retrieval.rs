use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use flux_provider::Provider;
use flux_types::SourceRef as FluxSourceRef;

/// Adapter that presents a standalone retriever as a Flux provider.
pub(crate) struct RetrievalProvider {
    inner: retriever::Retriever,
}

impl RetrievalProvider {
    /// Wrap a retriever for use with Flux provider APIs.
    pub(crate) fn new(inner: retriever::Retriever) -> Self {
        Self { inner }
    }

    fn map_source(src: &FluxSourceRef) -> anyhow::Result<retriever::SourceRef> {
        match src {
            FluxSourceRef::Http { url } => Ok(retriever::SourceRef::Http {
                url: url.as_ref().to_string(),
            }),
            FluxSourceRef::File { path } => Ok(retriever::SourceRef::File {
                path: std::path::PathBuf::from(path.as_ref()),
            }),
        }
    }
}

#[async_trait]
impl Provider for RetrievalProvider {
    async fn range_stream(
        &self,
        source: &FluxSourceRef,
        start: u64,
        end: u64,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<Bytes>>> {
        let src = Self::map_source(source)?;
        let cancel = CancellationToken::new();

        let s = self
            .inner
            .stream_range(src, retriever::ByteRange { start, end }, cancel)
            .map(|r| r.map_err(anyhow::Error::new));

        Ok(Box::pin(s))
    }

    fn is_range_stable(&self) -> bool {
        true
    }
}

/// Convenience constructor for Flux callsites.
pub(crate) fn provider_arc(inner: retriever::Retriever) -> Arc<dyn Provider> {
    Arc::new(RetrievalProvider::new(inner))
}
