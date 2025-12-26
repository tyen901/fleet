use anyhow::Result;

pub fn vec_to_md5_16(v: Vec<u8>) -> Result<[u8; 16]> {
    slice_to_md5_16(&v)
}

pub fn slice_to_md5_16(s: &[u8]) -> Result<[u8; 16]> {
    if s.len() != 16 {
        anyhow::bail!("expected 16-byte md5, got {} bytes", s.len());
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(s);
    Ok(out)
}

