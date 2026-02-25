use std::path::{Path, PathBuf};

pub async fn open_path(path: PathBuf) {
    let _ = tokio::task::spawn_blocking(move || {
        let target = open_target_path(&path).unwrap_or(path);
        let _ = open::that(target);
    })
    .await;
}

fn open_target_path(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }
    path.ancestors()
        .skip(1)
        .find(|candidate| candidate.exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::open_target_path;

    #[test]
    fn open_target_path_falls_back_to_existing_parent() {
        let base = std::env::current_dir().expect("current dir");
        let missing = base.join("definitely-missing-fleet-path").join("leaf");
        assert_eq!(open_target_path(&missing), Some(base));
    }
}
