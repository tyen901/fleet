use camino::{Utf8Path, Utf8PathBuf};
use manifest_types::Md5Digest;
use relative_path::RelativePath;
use rusqlite::params;
use tokio_rusqlite::Connection;

#[derive(thiserror::Error, Debug)]
pub enum IndexError {
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("async db error: {0}")]
    AsyncDb(#[from] tokio_rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt digest blob length {len}, expected 16")]
    CorruptDigest { len: usize },
}

#[derive(Clone, Debug)]
pub struct LocalIndex {
    inner: Connection,
}

#[derive(Debug, Clone)]
pub struct FileKey<'a> {
    pub mod_name: &'a str,
    pub rel_path: &'a RelativePath,
}

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub mtime_ns: i64,
    pub size: i64,
    pub expected: Md5Digest,
}

impl LocalIndex {
    pub async fn open(checkout_root: &Utf8Path) -> Result<Self, IndexError> {
        let dir = checkout_root.join(".fleet");
        std::fs::create_dir_all(dir.as_std_path())?;

        let db_path: Utf8PathBuf = dir.join("index.sqlite");
        let conn = Connection::open(db_path.as_std_path()).await?;

        conn.call(|conn| {
            Ok(conn.execute_batch(
                r#"
                PRAGMA journal_mode=WAL;
                CREATE TABLE IF NOT EXISTS files (
                  mod_name TEXT NOT NULL,
                  rel_path TEXT NOT NULL,
                  mtime_ns INTEGER NOT NULL,
                  size INTEGER NOT NULL,
                  expected BLOB NOT NULL,
                  PRIMARY KEY (mod_name, rel_path)
                );
                "#,
            )?)
        })
        .await?;

        Ok(Self { inner: conn })
    }

    pub async fn get(&self, key: FileKey<'_>) -> Result<Option<FileRecord>, IndexError> {
        let mod_name = key.mod_name.to_string();
        let rel = key.rel_path.as_str().to_string();

        let row = self
            .inner
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT mtime_ns, size, expected FROM files WHERE mod_name=?1 AND rel_path=?2",
                )?;
                Ok(stmt.query_row(params![mod_name, rel], |r| {
                    let mtime_ns: i64 = r.get(0)?;
                    let size: i64 = r.get(1)?;
                    let expected: Vec<u8> = r.get(2)?;
                    Ok((mtime_ns, size, expected))
                })?)
            })
            .await;

        match row {
            Ok((mtime_ns, size, expected)) => {
                if expected.len() != 16 {
                    return Err(IndexError::CorruptDigest {
                        len: expected.len(),
                    });
                }
                let mut buf = [0u8; 16];
                buf.copy_from_slice(&expected);
                Ok(Some(FileRecord {
                    mtime_ns,
                    size,
                    expected: Md5Digest::from_bytes(buf),
                }))
            }
            Err(tokio_rusqlite::Error::Rusqlite(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn upsert(
        &self,
        key: FileKey<'_>,
        mtime_ns: i64,
        size: i64,
        expected: Md5Digest,
    ) -> Result<(), IndexError> {
        let mod_name = key.mod_name.to_string();
        let rel = key.rel_path.as_str().to_string();

        self.inner
            .call(move |conn| {
                Ok(conn.execute(
                    r#"
                    INSERT INTO files(mod_name, rel_path, mtime_ns, size, expected)
                    VALUES(?1, ?2, ?3, ?4, ?5)
                    ON CONFLICT(mod_name, rel_path)
                    DO UPDATE SET mtime_ns=excluded.mtime_ns, size=excluded.size, expected=excluded.expected
                    "#,
                    params![
                        mod_name,
                        rel,
                        mtime_ns,
                        size,
                        expected.as_bytes().as_slice()
                    ],
                )?)
            })
            .await?;
        Ok(())
    }

    pub async fn delete(&self, key: FileKey<'_>) -> Result<(), IndexError> {
        let mod_name = key.mod_name.to_string();
        let rel = key.rel_path.as_str().to_string();
        self.inner
            .call(move |conn| {
                Ok(conn.execute(
                    "DELETE FROM files WHERE mod_name=?1 AND rel_path=?2",
                    params![mod_name, rel],
                )?)
            })
            .await?;
        Ok(())
    }
}
