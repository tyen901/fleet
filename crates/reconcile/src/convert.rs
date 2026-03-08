use anyhow::Result;

use flux_api::{DesiredState, FileSpec, SegmentSpec as ApiSegSpec};

pub(crate) fn desired_manifest_to_desired_state(
    m: &flux_manifest::DesiredManifest,
) -> Result<DesiredState> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for e in &m.entries {
        match e {
            flux_manifest::ManifestEntry::Dir { rel_path } => dirs.push(rel_path.clone()),
            flux_manifest::ManifestEntry::File(f) => {
                let segments = f
                    .segments
                    .iter()
                    .map(|s| ApiSegSpec {
                        source: s.source.clone(),
                        src_offset: s.src_offset,
                        len: s.len,
                        sig: s.signature.clone(),
                    })
                    .collect::<Vec<_>>();

                files.push(FileSpec {
                    rel_path: f.rel_path.clone(),
                    size_bytes: f.size_bytes,
                    mtime_ns: f.mtime_ns,
                    mode: f.mode,
                    segments,
                });
            }
        }
    }

    Ok(DesiredState {
        dirs,
        files,
        prune_paths: m.prune_paths.clone(),
    })
}
