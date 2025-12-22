use crate::ports::{EventSink, ModManifest, RemoteCapabilities, RemoteRepo, RemoteStream, RemoteStreamImpl, SyncEvent};
use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Mutex;

pub struct TestSink {
    events: Mutex<Vec<SyncEvent>>,
}

impl TestSink {
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    pub fn events(&self) -> Vec<SyncEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl Default for TestSink {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for TestSink {
    fn push(&self, ev: SyncEvent) {
        self.events.lock().unwrap().push(ev);
    }
}

pub struct MockRemoteRepo {
    caps: RemoteCapabilities,
    manifests: Mutex<HashMap<String, ModManifest>>,
    files: Mutex<HashMap<(String, String), Bytes>>,
    chunk_size: usize,
    range_calls: Mutex<Vec<(String, String, u64, u64)>>,
}

impl MockRemoteRepo {
    pub fn new(chunk_size: usize) -> Self {
        Self {
            caps: RemoteCapabilities {
                supports_ranges: true,
            },
            manifests: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
            chunk_size,
            range_calls: Mutex::new(Vec::new()),
        }
    }

    pub fn range_calls(&self) -> Vec<(String, String, u64, u64)> {
        self.range_calls.lock().unwrap().clone()
    }

    pub fn with_manifest(self, manifest: ModManifest) -> Self {
        self.manifests
            .lock()
            .unwrap()
            .insert(manifest.mod_id.clone(), manifest);
        self
    }

    pub fn with_file(self, mod_id: &str, rel_path: &str, bytes: Bytes) -> Self {
        self.files
            .lock()
            .unwrap()
            .insert((mod_id.to_string(), rel_path.to_string()), bytes);
        self
    }

    pub fn with_caps(self, caps: RemoteCapabilities) -> Self {
        let mut next = self;
        next.caps = caps;
        next
    }
}

struct BytesStream {
    bytes: Bytes,
    pos: usize,
    chunk_size: usize,
}

#[async_trait]
impl RemoteStreamImpl for BytesStream {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>> {
        if self.pos >= self.bytes.len() {
            return Ok(None);
        }
        let end = (self.pos + self.chunk_size).min(self.bytes.len());
        let out = self.bytes.slice(self.pos..end);
        self.pos = end;
        Ok(Some(out))
    }
}

#[async_trait]
impl RemoteRepo for MockRemoteRepo {
    async fn capabilities(&self) -> Result<RemoteCapabilities> {
        Ok(self.caps.clone())
    }

    async fn fetch_mod_manifest(&self, mod_id: &str) -> Result<ModManifest> {
        Ok(self.manifests.lock().unwrap().get(mod_id).cloned().unwrap())
    }

    async fn fetch_file(&self, mod_id: &str, rel_path: &str) -> Result<RemoteStream> {
        let b = self
            .files
            .lock()
            .unwrap()
            .get(&(mod_id.to_string(), rel_path.to_string()))
            .cloned()
            .unwrap();
        Ok(RemoteStream::new(Box::new(BytesStream {
            bytes: b,
            pos: 0,
            chunk_size: self.chunk_size,
        })))
    }

    async fn fetch_range(
        &self,
        mod_id: &str,
        rel_path: &str,
        offset: u64,
        len: u64,
    ) -> Result<RemoteStream> {
        self.range_calls.lock().unwrap().push((
            mod_id.to_string(),
            rel_path.to_string(),
            offset,
            len,
        ));
        let b = self
            .files
            .lock()
            .unwrap()
            .get(&(mod_id.to_string(), rel_path.to_string()))
            .cloned()
            .unwrap();
        let off = offset as usize;
        let end = (offset + len) as usize;
        let slice = b.slice(off..end.min(b.len()));
        Ok(RemoteStream::new(Box::new(BytesStream {
            bytes: slice,
            pos: 0,
            chunk_size: self.chunk_size,
        })))
    }
}
