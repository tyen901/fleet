use crate::api::{FileCacheEntry, LocalManifestSummary};
use crate::StorageError;

pub fn encode_manifest(manifest: &fleet_core::Manifest) -> Result<Vec<u8>, StorageError> {
    Ok(serde_json::to_vec(manifest)?)
}

pub fn decode_manifest(bytes: &[u8]) -> Result<fleet_core::Manifest, StorageError> {
    Ok(serde_json::from_slice(bytes)?)
}

pub fn encode_summary(summary: &[LocalManifestSummary]) -> Result<Vec<u8>, StorageError> {
    Ok(serde_json::to_vec(summary)?)
}

pub fn decode_summary(bytes: &[u8]) -> Result<Vec<LocalManifestSummary>, StorageError> {
    Ok(serde_json::from_slice(bytes)?)
}

pub fn encode_cache_entry(entry: &FileCacheEntry) -> Result<Vec<u8>, StorageError> {
    Ok(serde_json::to_vec(entry)?)
}

pub fn decode_cache_entry(bytes: &[u8]) -> Result<FileCacheEntry, StorageError> {
    Ok(serde_json::from_slice(bytes)?)
}
