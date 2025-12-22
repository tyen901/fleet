mod checksums;
mod digest;
pub mod formats;
mod model;
mod parts;
mod serde_helpers;

pub use checksums::{file_checksum_from_parts, mod_checksum_from_files};
pub use digest::{DigestError, Md5Digest};
pub use model::{
    FileManifest, ModManifest, PartManifest, RepoBasicAuth, RepoMod, RepoServer, RepoSpec,
};
pub use parts::{validate_parts, PartValidationError};
pub use serde_helpers::{deserialize_relpath, deserialize_u16_string_or_number};

impl RepoSpec {
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        let bytes = formats::strip_utf8_bom(bytes);
        formats::repo_json::parse(bytes)
    }
}

impl ModManifest {
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        formats::mod_manifest::parse_any(bytes)
    }
}
