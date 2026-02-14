use anyhow::{anyhow, Context, Result};
use std::path::Path;

pub async fn atomic_replace_file(tmp_path: &Path, dest_path: &Path) -> Result<()> {
    ensure_same_directory(tmp_path, dest_path)?;

    match tokio::fs::remove_file(dest_path).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            let _ = tokio::fs::remove_file(tmp_path).await;
            return Err(anyhow::Error::new(e))
                .with_context(|| format!("remove {}", dest_path.display()));
        }
    }

    if let Err(e) = tokio::fs::rename(tmp_path, dest_path)
        .await
        .with_context(|| {
            format!(
                "atomic replace {} -> {}",
                tmp_path.display(),
                dest_path.display()
            )
        })
    {
        let _ = tokio::fs::remove_file(tmp_path).await;
        return Err(e);
    }

    Ok(())
}

pub fn atomic_replace_file_sync(tmp_path: &Path, dest_path: &Path) -> Result<()> {
    ensure_same_directory(tmp_path, dest_path)?;

    match std::fs::remove_file(dest_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            let _ = std::fs::remove_file(tmp_path);
            return Err(anyhow::Error::new(e))
                .with_context(|| format!("remove {}", dest_path.display()));
        }
    }

    if let Err(e) = std::fs::rename(tmp_path, dest_path).with_context(|| {
        format!(
            "atomic replace {} -> {}",
            tmp_path.display(),
            dest_path.display()
        )
    }) {
        let _ = std::fs::remove_file(tmp_path);
        return Err(e);
    }

    Ok(())
}

fn ensure_same_directory(tmp_path: &Path, dest_path: &Path) -> Result<()> {
    let tmp_parent = tmp_path.parent().unwrap_or_else(|| Path::new(""));
    let dest_parent = dest_path.parent().unwrap_or_else(|| Path::new(""));
    if tmp_parent != dest_parent {
        return Err(anyhow!(
            "temp file must be in the same directory as destination"
        ));
    }
    Ok(())
}
