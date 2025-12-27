use std::path::{Path, PathBuf};

fn rs_files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file()
            && entry.path().extension().and_then(|e| e.to_str()) == Some("rs")
        {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

#[test]
fn sync_pipeline_never_mentions_repo_local_wire_files() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = crate_root.join("src");

    let mut bad = Vec::new();
    for p in rs_files_under(&src) {
        let bytes = std::fs::read(&p).expect("read source file");
        let s = String::from_utf8_lossy(&bytes);
        if s.contains("repo.json") || s.contains("mod.srf") {
            bad.push(p);
        }
    }

    assert!(
        bad.is_empty(),
        "repo-local wire filenames referenced in fleet_sync source: {bad:?}"
    );
}
