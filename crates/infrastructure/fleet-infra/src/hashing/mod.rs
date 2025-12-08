use byteorder::{LittleEndian, ReadBytesExt};
use camino::Utf8Path;
use fleet_core::{FilePart, FileType};
use md5::Context;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};

const MAX_PBO_STRING_LEN: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse PBO structure")]
    PboParse,
    #[error("String encoding error")]
    Utf8,
}

/// Compute the checksum string for a file using Swifty/Nimble logic.
pub fn compute_file_checksum(
    fs_path: &Utf8Path,
    logical_path: &Utf8Path,
) -> Result<String, ScanError> {
    let file = scan_file(fs_path, logical_path)?;
    Ok(file.checksum)
}

/// Scans a single file (PBO or Raw) and returns a fleet_core::File.
pub fn scan_file(
    fs_path: &Utf8Path,
    logical_path: &Utf8Path,
) -> Result<fleet_core::File, ScanError> {
    let extension = logical_path.extension().unwrap_or("").to_lowercase();

    if extension == "pbo" {
        scan_pbo(fs_path, logical_path)
    } else {
        scan_raw_file(fs_path, logical_path)
    }
}

// --- Raw File Logic ---

fn scan_raw_file(
    fs_path: &Utf8Path,
    logical_path: &Utf8Path,
) -> Result<fleet_core::File, ScanError> {
    let file = File::open(fs_path)?;
    let mut reader = BufReader::new(file);

    let mut parts = Vec::new();
    let mut pos: u64 = 0;

    // Nimble uses 5,000,000 byte chunks
    const CHUNK_SIZE: u64 = 5_000_000;

    // We can't easily predict file size if we just read stream,
    // but for the final struct we need total length.
    let total_len = fs_path.metadata()?.len();

    // Loop until EOF, hashing CHUNK_SIZE blocks into MD5 parts
    loop {
        let mut hasher = Context::new();
        let mut stream = reader.by_ref().take(CHUNK_SIZE);

        let pre_copy_pos = pos;
        let mut buf = [0u8; 8192];
        let mut copied = 0u64;
        loop {
            let n = stream.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.consume(&buf[..n]);
            copied += n as u64;
        }
        pos += copied;

        if copied == 0 {
            break;
        }

        let hash = format!("{:X}", hasher.finalize());

        // Nimble Naming Convention: "{filename}_{end_pos}"
        let file_name = logical_path.file_name().unwrap_or("unknown");

        parts.push(FilePart {
            path: format!("{}_{}", file_name, pos),
            length: copied,
            start: pre_copy_pos,
            checksum: hash,
        });
    }

    // Swifty hashes the Uppercase MD5 strings of the parts to get the final hash
    let mut hasher = Context::new();
    for part in &parts {
        hasher.consume(part.checksum.as_bytes());
    }

    Ok(fleet_core::File {
        path: logical_path.as_str().replace('\\', "/"),
        length: total_len,
        checksum: format!("{:X}", hasher.finalize()),
        file_type: FileType::File,
        parts,
    })
}

// --- PBO Logic ---

struct PboEntry {
    filename: String,
    data_size: u32,
    r#type: u32,
    // We don't actually need original_size, offset, timestamp for hashing
    // but we read them to advance the cursor correctly.
}

/// Reads the PBO header to determine header length and entry list.
/// This mimics `nimble/src/pbo.rs` logic exactly.
fn parse_pbo_metadata<R: BufRead + Seek>(input: &mut R) -> Result<(u64, Vec<PboEntry>), ScanError> {
    let mut entries = Vec::new();

    loop {
        let filename = read_null_terminated_string(input)?;

        let type_id = input.read_u32::<LittleEndian>()?;
        let _original_size = input.read_u32::<LittleEndian>()?;
        let _offset = input.read_u32::<LittleEndian>()?;
        let _timestamp = input.read_u32::<LittleEndian>()?;
        let data_size = input.read_u32::<LittleEndian>()?;

        if type_id == 0x56657273 {
            read_extensions(input)?;
            continue;
        }

        if type_id == 0 && filename.is_empty() {
            break;
        }

        entries.push(PboEntry {
            filename,
            data_size,
            r#type: type_id,
        });
    }

    let header_len = input.stream_position()?;
    Ok((header_len, entries))
}

fn read_extensions<R: BufRead>(input: &mut R) -> Result<HashMap<String, String>, ScanError> {
    let mut map = HashMap::new();
    loop {
        let key = read_null_terminated_string(input)?;
        if key.is_empty() {
            break;
        }
        let val = read_null_terminated_string(input)?;
        map.insert(key, val);
    }
    Ok(map)
}

fn read_null_terminated_string<R: BufRead>(input: &mut R) -> Result<String, ScanError> {
    let mut limited = input.take(MAX_PBO_STRING_LEN as u64);
    let mut buf = Vec::new();
    let bytes_read = limited.read_until(b'\0', &mut buf)?;

    if bytes_read == 0 {
        // EOF reached without data
        return Ok(String::new());
    }

    if buf.last() != Some(&b'\0') {
        // Did not find a null terminator within limit
        return Err(ScanError::PboParse);
    }

    buf.pop(); // remove null
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn scan_pbo(fs_path: &Utf8Path, logical_path: &Utf8Path) -> Result<fleet_core::File, ScanError> {
    let file = File::open(fs_path)?;
    let mut reader = BufReader::new(file);

    let (header_len, entries) = parse_pbo_metadata(&mut reader)?;

    let mut parts = Vec::new();
    let mut current_offset: u64 = 0;

    reader.seek(SeekFrom::Start(0))?;
    {
        let mut hasher = Context::new();
        let mut chunk = reader.by_ref().take(header_len);
        let mut buf = [0u8; 8192];
        loop {
            let n = chunk.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.consume(&buf[..n]);
        }

        parts.push(FilePart {
            path: "$$HEADER$$".to_string(),
            length: header_len,
            start: 0,
            checksum: format!("{:X}", hasher.finalize()),
        });
        current_offset += header_len;
    }

    for entry in entries.iter() {
        let size = entry.data_size as u64;

        let mut hasher = Context::new();
        let mut chunk = reader.by_ref().take(size);
        let mut buf = [0u8; 8192];
        let mut read_total = 0u64;
        loop {
            let n = chunk.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.consume(&buf[..n]);
            read_total += n as u64;
        }

        parts.push(FilePart {
            path: entry.filename.clone(),
            length: size,
            start: current_offset,
            checksum: format!("{:X}", hasher.finalize()),
        });

        current_offset += size;
    }

    let total_len = fs_path.metadata()?.len();
    let remaining = total_len.saturating_sub(current_offset);

    if remaining > 0 {
        let mut hasher = Context::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.consume(&buf[..n]);
        }

        parts.push(FilePart {
            path: "$$END$$".to_string(),
            length: remaining,
            start: current_offset,
            checksum: format!("{:X}", hasher.finalize()),
        });
    }

    let mut hasher = Context::new();
    for part in &parts {
        hasher.consume(part.checksum.as_bytes());
    }

    Ok(fleet_core::File {
        path: logical_path.as_str().replace('\\', "/"),
        length: total_len,
        checksum: format!("{:X}", hasher.finalize()),
        file_type: FileType::Pbo,
        parts,
    })
}
