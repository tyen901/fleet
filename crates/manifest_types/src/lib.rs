use md5::{Digest, Md5};
use relative_path::RelativePathBuf;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(thiserror::Error, Debug)]
pub enum DigestError {
    #[error("invalid hex digest: {0}")]
    InvalidHex(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Md5Digest {
    inner: [u8; 16],
}

impl Md5Digest {
    pub fn from_bytes(inner: [u8; 16]) -> Self {
        Self { inner }
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.inner
    }

    pub fn to_hex_upper(&self) -> String {
        hex::encode_upper(self.inner)
    }

    pub fn parse_hex(s: &str) -> Result<Self, DigestError> {
        let mut buf = [0u8; 16];
        hex::decode_to_slice(s, &mut buf).map_err(|_| DigestError::InvalidHex(s.to_string()))?;
        Ok(Self { inner: buf })
    }
}

impl fmt::Debug for Md5Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Md5Digest")
            .field(&self.to_hex_upper())
            .finish()
    }
}

impl Serialize for Md5Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex_upper())
    }
}

impl<'de> Deserialize<'de> for Md5Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse_hex(&s).map_err(serde::de::Error::custom)
    }
}

pub fn deserialize_relpath<'de, D>(deserializer: D) -> Result<RelativePathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let normalized = s.replace('\\', "/");
    Ok(RelativePathBuf::from(normalized))
}

pub fn deserialize_u16_string_or_number<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    struct V;

    impl<'de> serde::de::Visitor<'de> for V {
        type Value = u16;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "u16 or string containing a u16")
        }

        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<u16, E> {
            u16::try_from(v).map_err(|_| E::custom(format!("port out of range for u16: {v}")))
        }

        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<u16, E> {
            if v < 0 {
                return Err(E::custom(format!("port out of range for u16: {v}")));
            }
            u16::try_from(v as u64)
                .map_err(|_| E::custom(format!("port out of range for u16: {v}")))
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<u16, E> {
            v.parse::<u16>()
                .map_err(|e| E::custom(format!("invalid port string {v:?}: {e}")))
        }

        fn visit_string<E: serde::de::Error>(self, v: String) -> Result<u16, E> {
            self.visit_str(&v)
        }
    }

    deserializer.deserialize_any(V)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSpec {
    pub repo_name: String,
    pub checksum: String,

    pub required_mods: Vec<RepoMod>,
    pub optional_mods: Vec<RepoMod>,

    pub client_parameters: String,

    pub repo_basic_authentication: Option<RepoBasicAuth>,
    pub version: String,

    #[serde(default)]
    pub servers: Vec<RepoServer>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMod {
    pub mod_name: String,
    #[serde(rename = "checkSum")]
    pub checksum: Md5Digest,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoBasicAuth {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoServer {
    pub name: String,
    pub address: String,
    #[serde(deserialize_with = "deserialize_u16_string_or_number")]
    pub port: u16,
    pub password: String,
    pub battle_eye: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModManifest {
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "Checksum")]
    pub checksum: Md5Digest,
    #[serde(alias = "Files")]
    pub files: Vec<FileManifest>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileManifest {
    #[serde(deserialize_with = "deserialize_relpath", alias = "Path")]
    pub path: RelativePathBuf,
    #[serde(alias = "Length")]
    pub length: u64,
    #[serde(alias = "Checksum")]
    pub checksum: Md5Digest,
    #[serde(alias = "Parts")]
    pub parts: Vec<PartManifest>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartManifest {
    #[serde(alias = "Start")]
    pub start: u64,
    #[serde(alias = "Length")]
    pub length: u64,
    #[serde(alias = "Checksum")]
    pub checksum: Md5Digest,
}

pub fn file_checksum_from_parts(parts: &[PartManifest]) -> Md5Digest {
    let mut ctx = Md5::new();
    for p in parts {
        ctx.update(p.checksum.to_hex_upper().as_bytes());
    }
    Md5Digest::from_bytes(ctx.finalize().into())
}

pub fn mod_checksum_from_files(files: &[FileManifest]) -> Md5Digest {
    let mut files_sorted = files.to_vec();
    files_sorted.sort_by_key(|f| f.path.as_str().to_ascii_lowercase());

    let mut ctx = Md5::new();
    for f in files_sorted {
        ctx.update(f.checksum.to_hex_upper().as_bytes());
        let norm = f.path.as_str().replace('\\', "/").to_ascii_lowercase();
        ctx.update(norm.as_bytes());
    }
    Md5Digest::from_bytes(ctx.finalize().into())
}

#[derive(thiserror::Error, Debug)]
pub enum PartValidationError {
    #[error("parts are not contiguous")]
    NotContiguous,
    #[error("parts do not cover expected length")]
    LengthMismatch,
}

pub fn validate_parts(
    parts: &[PartManifest],
    expected_len: u64,
) -> Result<Vec<PartManifest>, PartValidationError> {
    if expected_len == 0 {
        return Ok(Vec::new());
    }
    let mut v = parts.to_vec();
    v.sort_by_key(|p| p.start);

    let mut pos = 0u64;
    for part in &v {
        if part.start != pos {
            return Err(PartValidationError::NotContiguous);
        }
        pos = pos.saturating_add(part.length);
    }
    if pos != expected_len {
        return Err(PartValidationError::LengthMismatch);
    }
    Ok(v)
}
