#[derive(Clone, Copy)]
pub(crate) struct Md5Checksummer;

impl fleet_sync::Checksummer for Md5Checksummer {
    fn algorithm_name(&self) -> &str {
        "md5"
    }

    fn hash_file(&self, path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
        use std::io::Read;

        let mut file = std::fs::File::open(path)?;
        let mut ctx = md5::Context::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            ctx.consume(&buf[..n]);
        }
        Ok(ctx.compute().0.to_vec())
    }

    fn hash_range(&self, path: &std::path::Path, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        use std::io::{Read, Seek};

        let mut file = std::fs::File::open(path)?;
        file.seek(std::io::SeekFrom::Start(offset))?;

        let mut remaining = len;
        let mut ctx = md5::Context::new();
        let mut buf = [0u8; 64 * 1024];
        while remaining > 0 {
            let want = usize::try_from(remaining.min(buf.len() as u64))?;
            let n = file.read(&mut buf[..want])?;
            if n == 0 {
                anyhow::bail!("unexpected EOF while hashing range");
            }
            ctx.consume(&buf[..n]);
            remaining -= n as u64;
        }
        Ok(ctx.compute().0.to_vec())
    }
}
