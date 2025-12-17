use byteorder::{LittleEndian, ReadBytesExt};
use std::{
    collections::HashMap,
    ffi::{CString, FromVecWithNulError},
    io::{BufRead, Seek},
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryType {
    Vers,
    Cprs,
    Enco,
    None,
}

#[derive(Debug, Clone)]
pub struct PboEntry {
    pub filename: String,
    pub entry_type: EntryType,
    pub original_size: u32,
    pub offset: u32,
    pub timestamp: u32,
    pub data_size: u32,
}

#[derive(Debug, Clone)]
pub struct PboMeta {
    pub header_len: u64,
    pub extensions: HashMap<String, String>,
    pub entries: Vec<PboEntry>,
}

#[derive(Debug, Error)]
pub enum PboError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unknown pbo entry type: {0:#x}")]
    UnknownEntryType(u32),

    #[error("invalid nul-terminated string: {0}")]
    BadCString(#[from] FromVecWithNulError),

    #[error("cstring exceeds maximum length of {0} bytes")]
    CStringTooLong(usize),

    #[error("unexpected EOF while reading cstring")]
    UnexpectedEofString,
}

const MAX_CSTRING: usize = 64 * 1024;

fn read_cstring<I: BufRead + Seek>(input: &mut I) -> Result<String, PboError> {
    let mut buf = Vec::new();
    loop {
        if buf.len() >= MAX_CSTRING {
            return Err(PboError::CStringTooLong(MAX_CSTRING));
        }
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Err(PboError::UnexpectedEofString);
        }
        if let Some(pos) = available.iter().position(|&b| b == 0) {
            buf.extend_from_slice(&available[..=pos]);
            input.consume(pos + 1);
            break;
        } else {
            buf.extend_from_slice(available);
            let n = available.len();
            input.consume(n);
        }
    }
    let cstring = CString::from_vec_with_nul(buf)?;
    Ok(cstring.to_string_lossy().to_string())
}

fn read_extensions<I: BufRead + Seek>(input: &mut I) -> Result<HashMap<String, String>, PboError> {
    let mut out = HashMap::new();
    loop {
        let key = read_cstring(input)?;
        if key.is_empty() {
            break;
        }
        let value = read_cstring(input)?;
        out.insert(key, value);
    }
    Ok(out)
}

impl PboEntry {
    fn read<I: BufRead + Seek>(input: &mut I) -> Result<Self, PboError> {
        let filename = read_cstring(input)?;
        let raw_type = input.read_u32::<LittleEndian>()?;

        let entry_type = match raw_type {
            0x56657273 => EntryType::Vers, // "Vers"
            0x43707273 => EntryType::Cprs, // "Cprs"
            0x456e6372 => EntryType::Enco, // "Encr"
            0x00000000 => EntryType::None,
            other => return Err(PboError::UnknownEntryType(other)),
        };

        let original_size = input.read_u32::<LittleEndian>()?;
        let offset = input.read_u32::<LittleEndian>()?;
        let timestamp = input.read_u32::<LittleEndian>()?;
        let data_size = input.read_u32::<LittleEndian>()?;

        Ok(Self {
            filename,
            entry_type,
            original_size,
            offset,
            timestamp,
            data_size,
        })
    }
}

impl PboMeta {
    pub fn read<I: BufRead + Seek>(input: &mut I) -> Result<Self, PboError> {
        let mut extensions = HashMap::new();
        let mut entries = Vec::new();

        loop {
            let entry = PboEntry::read(input)?;

            if entry.entry_type == EntryType::None && entry.filename.is_empty() {
                break;
            }

            if entry.entry_type == EntryType::Vers {
                extensions = read_extensions(input)?;
            }

            entries.push(entry);
        }

        let header_len = input.stream_position()?;

        Ok(Self {
            header_len,
            extensions,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_empty_pbo_header_with_sentinel() {
        let mut bytes = Vec::new();
        bytes.push(0);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        let mut cur = Cursor::new(bytes);
        let meta = PboMeta::read(&mut cur).unwrap();
        assert_eq!(meta.entries.len(), 0);
        assert!(meta.header_len > 0);
    }

    #[test]
    fn rejects_unknown_entry_type() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"a\0");
        bytes.extend_from_slice(&0x11111111u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        let mut cur = Cursor::new(bytes);
        let err = PboMeta::read(&mut cur).unwrap_err();
        assert!(matches!(err, PboError::UnknownEntryType(_)));
    }
}
