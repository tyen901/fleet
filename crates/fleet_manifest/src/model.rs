use crate::types::{FileMd5, ModId, PartMd5, RelPath};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModManifest {
    mod_id: ModId,
    files: Vec<FileEntry>,
}

impl ModManifest {
    pub fn mod_id(&self) -> &ModId {
        &self.mod_id
    }

    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    pub(crate) fn new_unchecked(mod_id: ModId, files: Vec<FileEntry>) -> Self {
        Self { mod_id, files }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    rel_path: RelPath,
    size: u64,
    file_md5: FileMd5,
    parts: Option<Vec<ManifestPart>>,
}

impl FileEntry {
    pub fn rel_path(&self) -> &RelPath {
        &self.rel_path
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn file_md5(&self) -> &FileMd5 {
        &self.file_md5
    }

    pub fn parts(&self) -> Option<&[ManifestPart]> {
        self.parts.as_deref()
    }

    pub(crate) fn new_unchecked(
        rel_path: RelPath,
        size: u64,
        file_md5: FileMd5,
        parts: Option<Vec<ManifestPart>>,
    ) -> Self {
        Self {
            rel_path,
            size,
            file_md5,
            parts,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestPart {
    pub offset: u64,
    pub len: u64,
    pub md5: PartMd5,
}

impl ManifestPart {
    pub fn end_exclusive(&self) -> u64 {
        self.offset.saturating_add(self.len)
    }
}
