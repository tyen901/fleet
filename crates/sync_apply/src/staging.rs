use crate::ApplyError;
use atomicwrites::{AtomicFile, Error as AtomicError, OverwriteBehavior};
use camino::{Utf8Path, Utf8PathBuf};
use manifest_types::Md5Digest;
use std::io::Write;

pub struct StagingFile {
    tmp_path: Utf8PathBuf,
    final_path: Utf8PathBuf,
    keep: bool,
}

impl StagingFile {
    pub fn new(final_path: &Utf8Path, expected: Option<Md5Digest>) -> Result<Self, ApplyError> {
        let dir = final_path
            .parent()
            .ok_or_else(|| std::io::Error::other("final_path has no parent"))?;

        let tmp_path = if let Some(expected) = expected {
            let base = final_path.file_name().unwrap_or("file").replace('/', "_");
            let name = format!(".fleet_tmp_{}_{}.part", expected.to_hex_upper(), base);
            dir.join(name)
        } else {
            let base = final_path.file_name().unwrap_or("file").replace('/', "_");
            let mut candidate = None;
            for i in 0..32u32 {
                let name = format!(".fleet_stage_{}_{}", base, i);
                let path = dir.join(name);
                if !path.exists() {
                    candidate = Some(path);
                    break;
                }
            }
            candidate.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "failed to create temp file after retries",
                )
            })?
        };

        if let Some(parent) = tmp_path.parent() {
            std::fs::create_dir_all(parent.as_std_path())?;
        }

        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(tmp_path.as_std_path())?;

        Ok(Self {
            tmp_path,
            final_path: final_path.to_owned(),
            keep: false,
        })
    }

    pub fn path(&self) -> &Utf8Path {
        &self.tmp_path
    }

    pub fn keep_on_drop(&mut self) {
        self.keep = true;
    }

    pub async fn replace(mut self) -> Result<(), ApplyError> {
        let tmp_path = self.tmp_path.clone();
        let final_path = self.final_path.clone();
        self.keep = true;

        tokio::task::spawn_blocking(move || {
            let mut src = std::fs::File::open(tmp_path.as_std_path())?;
            let atomic =
                AtomicFile::new(final_path.as_std_path(), OverwriteBehavior::AllowOverwrite);
            let res: Result<(), AtomicError<std::io::Error>> = atomic.write(|dst| {
                std::io::copy(&mut src, dst)?;
                dst.flush()?;
                Ok(())
            });

            match res {
                Ok(()) => Ok(()),
                Err(e) => Err(match e {
                    AtomicError::Internal(io) => io,
                    AtomicError::User(io) => io,
                }),
            }
        })
        .await
        .map_err(|e| ApplyError::AtomicReplace(format!("{e}")))?
        .map_err(|e| ApplyError::AtomicReplace(e.to_string()))?;

        let _ = tokio::fs::remove_file(self.tmp_path.as_std_path()).await;
        Ok(())
    }
}

impl Drop for StagingFile {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        let _ = std::fs::remove_file(self.tmp_path.as_std_path());
    }
}
