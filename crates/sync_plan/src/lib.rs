use manifest_types::{FileManifest, ModManifest};
use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncPlan {
    pub ops: Vec<Op>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Op {
    EnsureFileFromParts {
        mod_name: String,
        file: FileManifest,
    },

    DeleteFile {
        mod_name: String,
        rel_path: RelativePathBuf,
    },
}

impl SyncPlan {
    pub fn extend_mod_full(&mut self, manifest: &ModManifest) {
        for file in &manifest.files {
            self.ops.push(Op::EnsureFileFromParts {
                mod_name: manifest.name.clone(),
                file: file.clone(),
            });
        }
    }
}
