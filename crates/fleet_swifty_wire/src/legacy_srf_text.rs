use crate::Md5Digest;

#[derive(thiserror::Error, Debug)]
pub enum LegacyTextSrfError {
    #[error("utf8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("invalid legacy srf: {0}")]
    Invalid(&'static str),
    #[error("parse int: {0}")]
    Int(#[from] std::num::ParseIntError),
    #[error("digest: {0}")]
    Digest(#[from] crate::DigestError),
}

#[derive(Debug, Clone)]
pub struct LegacyTextMod {
    pub name: String,
    pub checksum: Md5Digest,
    pub files: Vec<LegacyTextFile>,
}

#[derive(Debug, Clone)]
pub struct LegacyTextFile {
    pub path: String,
    pub length: u64,
    pub checksum: Md5Digest,
    pub parts: Vec<LegacyTextPart>,
}

#[derive(Debug, Clone)]
pub struct LegacyTextPart {
    pub start: u64,
    pub length: u64,
    pub checksum: Md5Digest,
}

pub fn is_legacy_text_srf(bytes: &[u8]) -> bool {
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s.trim_start_matches('\u{feff}'),
        Err(_) => return false,
    };
    s.as_bytes().starts_with(b"ADDON")
}

pub fn parse_legacy_text_srf(bytes: &[u8]) -> Result<LegacyTextMod, LegacyTextSrfError> {
    let s = std::str::from_utf8(bytes)?.trim_start_matches('\u{feff}');
    let mut lines = s.lines().map(|l| l.trim_end_matches('\r'));

    let first = lines
        .next()
        .ok_or(LegacyTextSrfError::Invalid("missing first line"))?;
    let mut head = first.split(':');
    let magic = head
        .next()
        .ok_or(LegacyTextSrfError::Invalid("bad header"))?;
    if magic != "ADDON" {
        return Err(LegacyTextSrfError::Invalid("bad magic"));
    }

    let name = head
        .next()
        .ok_or(LegacyTextSrfError::Invalid("missing name"))?
        .to_string();
    let file_count: usize = head
        .next()
        .ok_or(LegacyTextSrfError::Invalid("missing file_count"))?
        .parse()?;
    let checksum_hex = head
        .next()
        .ok_or(LegacyTextSrfError::Invalid("missing checksum"))?;
    let checksum = Md5Digest::parse_hex(checksum_hex)?;

    let mut files = Vec::with_capacity(file_count);

    for _ in 0..file_count {
        let line = lines
            .next()
            .ok_or(LegacyTextSrfError::Invalid("missing file line"))?;
        let mut f = line.split(':');

        let _typ = f
            .next()
            .ok_or(LegacyTextSrfError::Invalid("missing file type"))?;
        let path = f
            .next()
            .ok_or(LegacyTextSrfError::Invalid("missing file path"))?
            .replace('\\', "/");
        let length: u64 = f
            .next()
            .ok_or(LegacyTextSrfError::Invalid("missing file length"))?
            .parse()?;
        let part_count: usize = f
            .next()
            .ok_or(LegacyTextSrfError::Invalid("missing part_count"))?
            .parse()?;
        let file_checksum = Md5Digest::parse_hex(
            f.next()
                .ok_or(LegacyTextSrfError::Invalid("missing file checksum"))?,
        )?;

        let mut parts = Vec::with_capacity(part_count);
        for _ in 0..part_count {
            let pline = lines
                .next()
                .ok_or(LegacyTextSrfError::Invalid("missing part line"))?;
            let mut p = pline.split(':');

            let _pname = p
                .next()
                .ok_or(LegacyTextSrfError::Invalid("missing part path"))?;
            let start: u64 = p
                .next()
                .ok_or(LegacyTextSrfError::Invalid("missing part start"))?
                .parse()?;
            let plen: u64 = p
                .next()
                .ok_or(LegacyTextSrfError::Invalid("missing part length"))?
                .parse()?;
            let pchk = Md5Digest::parse_hex(
                p.next()
                    .ok_or(LegacyTextSrfError::Invalid("missing part checksum"))?,
            )?;

            parts.push(LegacyTextPart {
                start,
                length: plen,
                checksum: pchk,
            });
        }

        files.push(LegacyTextFile {
            path,
            length,
            checksum: file_checksum,
            parts,
        });
    }

    Ok(LegacyTextMod {
        name,
        checksum,
        files,
    })
}
