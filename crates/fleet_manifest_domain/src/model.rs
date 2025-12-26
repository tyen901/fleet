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

    pub fn new(
        mod_id: impl Into<String>,
        files: Vec<FileEntry>,
    ) -> Result<Self, crate::ManifestError> {
        use std::collections::BTreeMap;

        let mod_id = ModId::new(mod_id)?;
        let mut files_by_path: BTreeMap<RelPath, FileEntry> = BTreeMap::new();
        for f in files {
            if let Some(parts) = f.parts.as_deref() {
                validate_parts(f.rel_path.as_str(), f.size, parts)?;
            }
            let rel_path = f.rel_path.clone();
            if files_by_path.insert(rel_path.clone(), f).is_some() {
                return Err(crate::ManifestError::DuplicateFile(
                    rel_path.as_str().to_string(),
                ));
            }
        }
        Ok(Self::new_unchecked(
            mod_id,
            files_by_path.into_values().collect(),
        ))
    }
}

fn validate_parts(
    rel_path: &str,
    file_size: u64,
    parts: &[ManifestPart],
) -> Result<(), crate::ManifestError> {
    if parts.is_empty() {
        return Err(crate::ManifestError::InvalidParts {
            rel_path: rel_path.to_string(),
            msg: "parts present but empty".into(),
        });
    }

    let mut expected_offset = 0u64;
    for (idx, part) in parts.iter().enumerate() {
        if part.offset != expected_offset {
            return Err(crate::ManifestError::InvalidParts {
                rel_path: rel_path.to_string(),
                msg: format!(
                    "non-contiguous at index {idx}: expected offset {expected_offset}, got {}",
                    part.offset
                ),
            });
        }
        let end_exclusive = part.offset.checked_add(part.len).ok_or_else(|| {
            crate::ManifestError::InvalidParts {
                rel_path: rel_path.to_string(),
                msg: "part offset+length overflow".into(),
            }
        })?;
        expected_offset = end_exclusive;
        if expected_offset > file_size {
            return Err(crate::ManifestError::InvalidParts {
                rel_path: rel_path.to_string(),
                msg: format!("part exceeds file size: end {expected_offset} > size {file_size}"),
            });
        }
    }

    if expected_offset != file_size {
        return Err(crate::ManifestError::InvalidParts {
            rel_path: rel_path.to_string(),
            msg: format!("parts do not cover file: covered {expected_offset}, size {file_size}"),
        });
    }

    Ok(())
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

    pub fn new(
        rel_path: RelPath,
        size: u64,
        file_md5: FileMd5,
        parts: Option<Vec<ManifestPart>>,
    ) -> Result<Self, crate::ManifestError> {
        if let Some(parts) = parts.as_deref() {
            validate_parts(rel_path.as_str(), size, parts)?;
        }
        Ok(Self::new_unchecked(rel_path, size, file_md5, parts))
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
