use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Component, Path};

use crate::errors::ManifestError;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModId(String);

impl ModId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn new(s: impl Into<String>) -> Result<Self, ManifestError> {
        let s = s.into();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ManifestError::InvalidModId("empty".into()));
        }
        Ok(Self(trimmed.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(String);

impl RelPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn new(raw: &str) -> Result<Self, ManifestError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(ManifestError::InvalidRelPath("empty".into()));
        }
        if raw.contains('\\') {
            return Err(ManifestError::InvalidRelPath(
                "backslashes are not allowed; use forward slashes".into(),
            ));
        }
        if raw.starts_with('/') {
            return Err(ManifestError::InvalidRelPath(format!(
                "absolute path: {raw}"
            )));
        }
        if raw.contains('\0') {
            return Err(ManifestError::InvalidRelPath("contains NUL".into()));
        }

        let p = Path::new(raw);
        let mut out = Vec::<Cow<'_, str>>::new();

        for comp in p.components() {
            match comp {
                Component::Normal(os) => {
                    let s = os.to_string_lossy();
                    if s == "." || s.is_empty() {
                        continue;
                    }
                    if s.contains(':') {
                        return Err(ManifestError::InvalidRelPath(format!(
                            "invalid component: {s}"
                        )));
                    }
                    out.push(s);
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(ManifestError::InvalidRelPath(format!(
                        "parent traversal: {raw}"
                    )));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ManifestError::InvalidRelPath(format!(
                        "absolute/prefix path: {raw}"
                    )));
                }
            }
        }

        if out.is_empty() {
            return Err(ManifestError::InvalidRelPath(format!(
                "no components: {raw}"
            )));
        }

        Ok(Self(out.join("/")))
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Digest16<Tag> {
    bytes: [u8; 16],
    _tag: PhantomData<Tag>,
}

impl<Tag> Digest16<Tag> {
    pub fn new(bytes: [u8; 16]) -> Self {
        Self {
            bytes,
            _tag: PhantomData,
        }
    }

    pub fn bytes(&self) -> &[u8; 16] {
        &self.bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileDigestTag {}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartDigestTag {}

pub type FileMd5 = Digest16<FileDigestTag>;
pub type PartMd5 = Digest16<PartDigestTag>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchRange {
    pub offset: u64,
    pub len: u64,
}

impl FetchRange {
    pub fn end_exclusive(&self) -> u64 {
        self.offset.saturating_add(self.len)
    }
}
