pub mod arma;
pub mod core;
pub mod swifty;

pub use core::{DigestError, Md5Digest};
pub use swifty::checksums::{file_checksum_from_parts, mod_checksum_from_files};
pub use swifty::model::{
    FileManifest, ModManifest, PartManifest, RepoBasicAuth, RepoMod, RepoServer, RepoSpec,
};
pub use swifty::validation::{validate_parts, PartValidationError};

impl RepoSpec {
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        let bytes = swifty::formats::strip_utf8_bom(bytes);
        swifty::formats::repo_json::parse(bytes)
    }
}

impl ModManifest {
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        swifty::formats::mod_manifest::parse_any(bytes)
    }
}
