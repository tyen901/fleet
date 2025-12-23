use std::collections::HashMap;
use std::io::{BufRead, Read, Seek, SeekFrom};

#[derive(Debug, PartialEq, Eq)]
pub enum EntryType {
    Vers,
    Cprs,
    Enco,
    None,
    Other(u32),
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct PboEntry {
    pub filename: String,
    pub entry_type: EntryType,
    pub data_size: u32,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct PboMeta {
    pub header_len: u64,
    pub extensions: HashMap<String, String>,
    pub entries: Vec<PboEntry>,
}

fn read_cstring<R: BufRead>(r: &mut R) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        r.read_exact(&mut b)?;
        if b[0] == 0 {
            break;
        }
        out.push(b[0]);
        if out.len() > 1024 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "cstring too long",
            ));
        }
    }
    Ok(out)
}

fn read_cstring_string<R: BufRead>(r: &mut R) -> std::io::Result<String> {
    let v = read_cstring(r)?;
    Ok(String::from_utf8_lossy(&v).to_string())
}

fn read_u32_le<R: Read>(r: &mut R) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn map_type(t: u32) -> EntryType {
    match t {
        0x5665_7273 => EntryType::Vers,
        0x4370_7273 => EntryType::Cprs,
        0x456e_6372 => EntryType::Enco,
        0x0000_0000 => EntryType::None,
        other => EntryType::Other(other),
    }
}

fn read_extensions<R: BufRead>(r: &mut R) -> std::io::Result<HashMap<String, String>> {
    let mut m = HashMap::new();
    loop {
        let key = read_cstring_string(r)?;
        if key.is_empty() {
            break;
        }
        let val = read_cstring_string(r)?;
        m.insert(key, val);
    }
    Ok(m)
}

pub fn read_pbo_meta<R: BufRead + Seek>(r: &mut R) -> std::io::Result<PboMeta> {
    r.seek(SeekFrom::Start(0))?;

    let mut extensions = HashMap::new();
    let mut entries = Vec::new();

    loop {
        let filename = read_cstring_string(r)?;
        let t_raw = read_u32_le(r)?;
        let entry_type = map_type(t_raw);

        let _original_size = read_u32_le(r)?;
        let _offset = read_u32_le(r)?;
        let _timestamp = read_u32_le(r)?;
        let data_size = read_u32_le(r)?;

        if entry_type == EntryType::None && filename.is_empty() {
            break;
        }

        if entry_type == EntryType::Vers {
            extensions = read_extensions(r)?;
        }

        entries.push(PboEntry {
            filename,
            entry_type,
            data_size,
        });
    }

    let header_len = r.stream_position()?;
    Ok(PboMeta {
        header_len,
        extensions,
        entries,
    })
}

#[derive(thiserror::Error, Debug)]
pub enum PboPartitionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid pbo: {0}")]
    Invalid(&'static str),
}

/// Swifty/Nimble-compatible PBO partitioning.
///
/// Returns the list of `(start, length)` parts (header, per-entry, tail).
pub fn partition_pbo<R: BufRead + Seek>(
    reader: &mut R,
    file_len: u64,
) -> Result<Vec<(u64, u64)>, PboPartitionError> {
    let meta = read_pbo_meta(reader).map_err(PboPartitionError::Io)?;
    if meta.header_len > file_len {
        return Err(PboPartitionError::Invalid("header_len exceeds file length"));
    }
    if meta.entries.first().is_some_and(|e| e.data_size != 0) {
        return Err(PboPartitionError::Invalid(
            "pbo first entry must have data_size == 0",
        ));
    }

    reader.seek(SeekFrom::Start(0))?;

    let mut parts: Vec<(u64, u64)> = Vec::new();
    let mut offset = 0u64;

    parts.push((0, meta.header_len));
    offset += meta.header_len;

    for entry in meta.entries.iter().skip(1) {
        let len = entry.data_size as u64;
        parts.push((offset, len));
        offset = offset.saturating_add(len);
    }

    if offset > file_len {
        return Err(PboPartitionError::Invalid(
            "pbo parts offset exceeded file length",
        ));
    }

    let remaining = file_len - offset;
    parts.push((offset, remaining));
    offset += remaining;

    if offset != file_len {
        return Err(PboPartitionError::Invalid("parts do not cover file length"));
    }

    Ok(parts)
}
